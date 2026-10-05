//! OpenAI-compatible chat completions client with SSE streaming.
//!
//! Works with llama.cpp `llama-server`, Ollama (`/v1`), LM Studio, vLLM and
//! other compatible servers. Handles streamed text, `reasoning_content`
//! (thinking), incremental tool-call deltas, and text-embedded tool calls.

use crate::toolparse::{extract_text_tool_calls, split_thinking};
use crate::{ChatRequest, ChatResponse, EventSink, Message, ModelProvider, ProviderInfo, Role, StreamEvent, ToolCall};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use veyra_protocol::TokenUsage;

#[derive(Debug, Clone)]
pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
    provider_label: String,
    context_length: Option<u32>,
}

impl OpenAiCompatProvider {
    pub fn new(base_url: &str, model: &str) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // No overall timeout: long generations on slow hardware are fine.
            .build()
            .expect("http client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key: None,
            provider_label: "openai-compatible".into(),
            context_length: None,
        }
    }

    pub fn with_api_key(mut self, key: Option<String>) -> Self {
        self.api_key = key.filter(|k| !k.is_empty());
        self
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.provider_label = label.to_string();
        self
    }

    pub fn with_context_length(mut self, n: Option<u32>) -> Self {
        self.context_length = n;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// List model ids served by the endpoint (`GET /models`).
    pub async fn list_models(&self) -> anyhow::Result<Vec<String>> {
        let mut req = self.client.get(format!("{}/models", self.base_url));
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }
        let v: Value = req.send().await?.error_for_status()?.json().await?;
        Ok(v.get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    fn body(&self, req: &ChatRequest) -> Value {
        let messages: Vec<Value> = req.messages.iter().map(message_json).collect();
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        if !req.tools.is_empty() {
            body["tools"] = Value::Array(
                req.tools
                    .iter()
                    .map(|t| {
                        json!({"type": "function", "function": {
                            "name": t.name, "description": t.description, "parameters": t.parameters
                        }})
                    })
                    .collect(),
            );
            body["tool_choice"] = json!("auto");
        }
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = req.max_tokens {
            body["max_tokens"] = json!(m);
        }
        body
    }
}

fn message_json(m: &Message) -> Value {
    let role = match m.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut v = json!({"role": role, "content": m.content});
    if !m.tool_calls.is_empty() {
        v["tool_calls"] = Value::Array(
            m.tool_calls
                .iter()
                .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments}}))
                .collect(),
        );
    }
    if let Some(id) = &m.tool_call_id {
        v["tool_call_id"] = json!(id);
    }
    if let Some(n) = &m.name {
        if m.role == Role::Tool {
            v["name"] = json!(n);
        }
    }
    v
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[async_trait::async_trait]
impl ModelProvider for OpenAiCompatProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            provider: self.provider_label.clone(),
            model: self.model.clone(),
            endpoint: self.base_url.clone(),
            local: veyra_endpoint_is_local(&self.base_url),
            context_length: self.context_length,
        }
    }

    async fn chat(&self, req: ChatRequest, on_event: EventSink<'_>, cancel: &CancellationToken) -> anyhow::Result<ChatResponse> {
        let known: Vec<String> = req.tools.iter().map(|t| t.name.clone()).collect();
        let body = self.body(&req);

        // Retry connection failures briefly (runtime may still be loading).
        let mut attempt = 0;
        let resp = loop {
            let mut r = self.client.post(format!("{}/chat/completions", self.base_url)).json(&body);
            if let Some(k) = &self.api_key {
                r = r.bearer_auth(k);
            }
            match r.send().await {
                Ok(resp) => break resp,
                Err(e) if e.is_connect() && attempt < 3 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(500 * attempt)).await;
                }
                Err(e) => return Err(anyhow::anyhow!("model endpoint {} unreachable: {e}", self.base_url)),
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("model endpoint returned {status}: {}", text.chars().take(600).collect::<String>());
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut out = ChatResponse::default();
        let mut calls: BTreeMap<u64, PartialCall> = BTreeMap::new();
        let mut in_think = false;

        'outer: loop {
            let chunk = tokio::select! {
                c = stream.next() => c,
                _ = cancel.cancelled() => anyhow::bail!("cancelled"),
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim_end_matches('\r').to_string();
                buf.drain(..=pos);
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" {
                    break 'outer;
                }
                let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                if let Some(err) = v.get("error") {
                    anyhow::bail!("model error: {err}");
                }
                if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                    out.usage = Some(TokenUsage {
                        prompt_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                        completion_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0),
                    });
                }
                let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else { continue };
                if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
                    out.finish_reason = Some(fr.to_string());
                }
                let Some(delta) = choice.get("delta").or_else(|| choice.get("message")) else { continue };
                for key in ["reasoning_content", "reasoning"] {
                    if let Some(r) = delta.get(key).and_then(Value::as_str) {
                        if !r.is_empty() {
                            out.reasoning.push_str(r);
                            on_event(StreamEvent::Reasoning(r.to_string()));
                        }
                    }
                }
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        out.content.push_str(text);
                        // Route inline <think> blocks to the reasoning stream.
                        let mut rest = text;
                        while !rest.is_empty() {
                            if in_think {
                                match rest.find("</think>") {
                                    Some(i) => {
                                        on_event(StreamEvent::Reasoning(rest[..i].to_string()));
                                        rest = &rest[i + 8..];
                                        in_think = false;
                                    }
                                    None => {
                                        on_event(StreamEvent::Reasoning(rest.to_string()));
                                        rest = "";
                                    }
                                }
                            } else {
                                match rest.find("<think>") {
                                    Some(i) => {
                                        if i > 0 {
                                            on_event(StreamEvent::Text(rest[..i].to_string()));
                                        }
                                        rest = &rest[i + 7..];
                                        in_think = true;
                                    }
                                    None => {
                                        on_event(StreamEvent::Text(rest.to_string()));
                                        rest = "";
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
                    for tc in tcs {
                        let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(calls.len() as u64);
                        let entry = calls.entry(idx).or_default();
                        if let Some(id) = tc.get("id").and_then(Value::as_str) {
                            entry.id = id.to_string();
                        }
                        if let Some(f) = tc.get("function") {
                            if let Some(n) = f.get("name").and_then(Value::as_str) {
                                if entry.name.is_empty() {
                                    on_event(StreamEvent::ToolCall(n.to_string()));
                                }
                                entry.name.push_str(n);
                            }
                            match f.get("arguments") {
                                Some(Value::String(a)) => entry.arguments.push_str(a),
                                Some(obj @ Value::Object(_)) => entry.arguments = obj.to_string(),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        out.tool_calls = calls
            .into_values()
            .enumerate()
            .filter(|(_, c)| !c.name.is_empty())
            .map(|(i, c)| ToolCall {
                id: if c.id.is_empty() { format!("call_{i}") } else { c.id },
                name: c.name,
                arguments: c.arguments,
            })
            .collect();

        let (content, thinking) = split_thinking(&out.content);
        if !thinking.is_empty() && out.reasoning.is_empty() {
            out.reasoning = thinking;
        }
        out.content = content;
        if out.tool_calls.is_empty() && !known.is_empty() {
            let names: Vec<&str> = known.iter().map(String::as_str).collect();
            let (rest, calls) = extract_text_tool_calls(&out.content, &names);
            if !calls.is_empty() {
                out.content = rest;
                out.tool_calls = calls;
            }
        }
        Ok(out)
    }
}

/// Same rule as veyra-core's `endpoint_is_local`, duplicated to keep this
/// crate independent of configuration.
fn veyra_endpoint_is_local(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or("");
    let host = if authority.starts_with('[') {
        authority.split(']').next().unwrap_or("").trim_start_matches('[').to_string()
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolDef;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Minimal HTTP server that replies with a canned SSE body.
    async fn serve(body: String) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let h = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut req = vec![0u8; 65536];
            let mut total = 0;
            loop {
                let n = sock.read(&mut req[total..]).await.unwrap();
                total += n;
                let s = String::from_utf8_lossy(&req[..total]);
                if let Some(hdr_end) = s.find("\r\n\r\n") {
                    let len: usize = s
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                        .unwrap_or(0);
                    if total >= hdr_end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&req[..total]).into_owned()
        });
        (format!("http://{addr}/v1"), h)
    }

    fn sse(chunks: &[Value]) -> String {
        let mut s: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
        s.push_str("data: [DONE]\n\n");
        s
    }

    #[tokio::test]
    async fn streams_text_reasoning_and_usage() {
        let body = sse(&[
            json!({"choices":[{"delta":{"reasoning_content":"thinking..."}}]}),
            json!({"choices":[{"delta":{"content":"Hel"}}]}),
            json!({"choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3}}),
        ]);
        let (url, server) = serve(body).await;
        let p = OpenAiCompatProvider::new(&url, "test-model");
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let sink = move |e: StreamEvent| ev.lock().unwrap().push(e);
        let req = ChatRequest {
            messages: vec![Message::user("hi")],
            ..Default::default()
        };
        let r = p.chat(req, &sink, &CancellationToken::new()).await.unwrap();
        assert_eq!(r.content, "Hello");
        assert_eq!(r.reasoning, "thinking...");
        assert_eq!(r.usage.unwrap().prompt_tokens, 12);
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
        let evs = events.lock().unwrap();
        assert!(evs.contains(&StreamEvent::Text("Hel".into())));
        let sent = server.await.unwrap();
        assert!(sent.contains("\"stream\":true"));
        assert!(sent.starts_with("POST /v1/chat/completions"));
    }

    #[tokio::test]
    async fn assembles_streamed_tool_calls() {
        let body = sse(&[
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"grep","arguments":""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"pattern\":"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"auth\"}"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c2","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":"tool_calls"}]}),
        ]);
        let (url, server) = serve(body).await;
        let p = OpenAiCompatProvider::new(&url, "m");
        let req = ChatRequest {
            messages: vec![Message::user("hi")],
            tools: vec![ToolDef { name: "grep".into(), description: "d".into(), parameters: json!({"type":"object"}) }],
            ..Default::default()
        };
        let r = p.chat(req, &|_| {}, &CancellationToken::new()).await.unwrap();
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].name, "grep");
        assert_eq!(r.tool_calls[0].parsed_arguments().unwrap()["pattern"], "auth");
        assert_eq!(r.tool_calls[1].id, "c2");
        let sent = server.await.unwrap();
        assert!(sent.contains("\"tools\""));
    }

    #[tokio::test]
    async fn recovers_text_tool_calls_and_inline_thinking() {
        let body = sse(&[
            json!({"choices":[{"delta":{"content":"<think>need to search</think>"}}]}),
            json!({"choices":[{"delta":{"content":"<tool_call>{\"name\":\"grep\",\"arguments\":{\"pattern\":\"x\"}}</tool_call>"}}]}),
        ]);
        let (url, _server) = serve(body).await;
        let p = OpenAiCompatProvider::new(&url, "m");
        let req = ChatRequest {
            messages: vec![Message::user("hi")],
            tools: vec![ToolDef { name: "grep".into(), description: "d".into(), parameters: json!({}) }],
            ..Default::default()
        };
        let r = p.chat(req, &|_| {}, &CancellationToken::new()).await.unwrap();
        assert_eq!(r.reasoning, "need to search");
        assert_eq!(r.tool_calls.len(), 1);
        assert!(r.content.is_empty());
    }

    #[test]
    fn locality() {
        assert!(OpenAiCompatProvider::new("http://127.0.0.1:8080/v1", "m").info().local);
        assert!(!OpenAiCompatProvider::new("https://api.example.com/v1", "m").info().local);
    }
}
