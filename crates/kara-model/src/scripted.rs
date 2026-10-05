//! Deterministic model for tests, CI and offline harness checks.
//!
//! A `ScriptedProvider` replays a list of responses, or computes each response
//! from the request with a closure. It records every request so tests can
//! assert on what the agent sent (tool results, working memory, warnings).

use crate::{
    ChatRequest, ChatResponse, EventSink, ModelProvider, ProviderInfo, StreamEvent, ToolCall,
};
use kara_protocol::TokenUsage;
use serde::Deserialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

type Responder = dyn Fn(&ChatRequest, usize) -> ChatResponse + Send + Sync;

pub struct ScriptedProvider {
    queue: Mutex<VecDeque<ChatResponse>>,
    responder: Option<Box<Responder>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    name: String,
}

/// One scripted step as stored in JSON script files.
#[derive(Debug, Deserialize)]
pub struct ScriptStep {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ScriptCall>,
}

#[derive(Debug, Deserialize)]
pub struct ScriptCall {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

pub fn text(content: &str) -> ChatResponse {
    ChatResponse {
        content: content.to_string(),
        ..Default::default()
    }
}

pub fn call(name: &str, args: Value) -> ChatResponse {
    calls(&[(name, args)])
}

pub fn calls(list: &[(&str, Value)]) -> ChatResponse {
    ChatResponse {
        tool_calls: list
            .iter()
            .enumerate()
            .map(|(i, (n, a))| ToolCall {
                id: format!("scripted_{i}"),
                name: n.to_string(),
                arguments: a.to_string(),
            })
            .collect(),
        ..Default::default()
    }
}

impl ScriptedProvider {
    pub fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            queue: Mutex::new(responses.into()),
            responder: None,
            requests: Arc::new(Mutex::new(Vec::new())),
            name: "scripted".into(),
        }
    }

    pub fn from_fn(
        f: impl Fn(&ChatRequest, usize) -> ChatResponse + Send + Sync + 'static,
    ) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            responder: Some(Box::new(f)),
            requests: Arc::new(Mutex::new(Vec::new())),
            name: "scripted".into(),
        }
    }

    pub fn from_script_json(text: &str) -> anyhow::Result<Self> {
        let steps: Vec<ScriptStep> = serde_json::from_str(text)?;
        let responses = steps
            .into_iter()
            .map(|s| {
                let mut r = ChatResponse {
                    content: s.content,
                    ..Default::default()
                };
                r.tool_calls = s
                    .tool_calls
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| ToolCall {
                        id: format!("scripted_{i}"),
                        name: c.name,
                        arguments: c.arguments.to_string(),
                    })
                    .collect();
                r
            })
            .collect();
        Ok(Self::new(responses))
    }

    pub fn requests(&self) -> Arc<Mutex<Vec<ChatRequest>>> {
        self.requests.clone()
    }
}

#[async_trait::async_trait]
impl ModelProvider for ScriptedProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            provider: self.name.clone(),
            model: "scripted".into(),
            endpoint: "in-process".into(),
            local: true,
            context_length: Some(32768),
        }
    }

    async fn chat(
        &self,
        req: ChatRequest,
        on_event: EventSink<'_>,
        cancel: &CancellationToken,
    ) -> anyhow::Result<ChatResponse> {
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled");
        }
        let n = {
            let mut r = self.requests.lock().unwrap();
            r.push(req.clone());
            r.len() - 1
        };
        let mut resp = match &self.responder {
            Some(f) => f(&req, n),
            None => self
                .queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| text("(script exhausted)")),
        };
        for word in resp.content.split_inclusive(' ') {
            on_event(StreamEvent::Text(word.to_string()));
        }
        for c in &resp.tool_calls {
            on_event(StreamEvent::ToolCall(c.name.clone()));
        }
        let prompt_chars: usize = req.messages.iter().map(|m| m.content.len()).sum();
        resp.usage = Some(TokenUsage {
            prompt_tokens: (prompt_chars / 4) as u64,
            completion_tokens: (resp.content.len() / 4) as u64 + 1,
        });
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    #[tokio::test]
    async fn replays_and_records() {
        let p = ScriptedProvider::new(vec![
            call("grep", serde_json::json!({"pattern": "x"})),
            text("done"),
        ]);
        let req = ChatRequest {
            messages: vec![Message::user("go")],
            ..Default::default()
        };
        let a = p
            .chat(req.clone(), &|_| {}, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(a.tool_calls[0].name, "grep");
        let b = p
            .chat(req.clone(), &|_| {}, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(b.content, "done");
        assert_eq!(p.requests().lock().unwrap().len(), 2);
    }

    #[test]
    fn json_scripts() {
        let p = ScriptedProvider::from_script_json(
            r#"[{"tool_calls":[{"name":"read_file","arguments":{"path":"a"}}]},{"content":"ok"}]"#,
        )
        .unwrap();
        assert_eq!(p.queue.lock().unwrap().len(), 2);
    }
}
