//! Tolerant tool-call handling for local models.
//!
//! Runtimes with native tool calling return structured `tool_calls`. Some
//! models (or runtimes without a matching chat template) instead emit calls as
//! text, e.g. Qwen/Hermes `<tool_call>{...}</tool_call>` blocks or fenced
//! JSON. These helpers recover such calls and normalise arguments.

use crate::ToolCall;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

pub fn parse_arguments(raw: &str) -> Result<Value, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    match serde_json::from_str::<Value>(t) {
        Ok(Value::String(inner)) => parse_arguments(&inner),
        Ok(v @ Value::Object(_)) => Ok(v),
        Ok(other) => Err(format!("tool arguments must be a JSON object, got {other}")),
        Err(e) => {
            // Accept a complete object followed by junk (some models append text).
            let mut de = serde_json::Deserializer::from_str(t).into_iter::<Value>();
            if let Some(Ok(v @ Value::Object(_))) = de.next() {
                return Ok(v);
            }
            Err(format!("tool arguments are not valid JSON: {e}"))
        }
    }
}

fn re_tool_call() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<tool_call>\s*(.*?)\s*(?:</tool_call>|$)").expect("regex"))
}

fn re_think() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<think>(.*?)(?:</think>|$)").expect("regex"))
}

/// Split `<think>...</think>` blocks out of content.
pub fn split_thinking(content: &str) -> (String, String) {
    let mut reasoning = String::new();
    for c in re_think().captures_iter(content) {
        reasoning.push_str(c[1].trim());
        reasoning.push('\n');
    }
    let cleaned = re_think().replace_all(content, "");
    // A stray closing tag (thinking started in an earlier chunk).
    let cleaned = match cleaned.find("</think>") {
        Some(i) => {
            reasoning.push_str(&cleaned[..i]);
            cleaned[i + "</think>".len()..].to_string()
        }
        None => cleaned.into_owned(),
    };
    (cleaned.trim().to_string(), reasoning.trim().to_string())
}

/// Recover tool calls written as text. Returns remaining content and calls.
pub fn extract_text_tool_calls(content: &str, known_tools: &[&str]) -> (String, Vec<ToolCall>) {
    let mut calls = Vec::new();
    let mut remaining = content.to_string();

    for c in re_tool_call().captures_iter(content) {
        if let Some(call) = call_from_json(&c[1], known_tools, calls.len()) {
            calls.push(call);
        }
    }
    if !calls.is_empty() {
        remaining = re_tool_call().replace_all(content, "").trim().to_string();
        return (remaining, calls);
    }

    // Fenced or bare JSON objects naming a known tool.
    static FENCE: OnceLock<Regex> = OnceLock::new();
    let fence = FENCE.get_or_init(|| Regex::new(r"(?s)```(?:json|tool_call)?\s*(\{.*?\})\s*```").expect("regex"));
    for c in fence.captures_iter(content) {
        if let Some(call) = call_from_json(&c[1], known_tools, calls.len()) {
            calls.push(call);
        }
    }
    if !calls.is_empty() {
        remaining = fence.replace_all(content, "").trim().to_string();
        return (remaining, calls);
    }
    let trimmed = content.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        if let Some(call) = call_from_json(trimmed, known_tools, 0) {
            return (String::new(), vec![call]);
        }
    }
    (remaining, calls)
}

fn call_from_json(text: &str, known: &[&str], n: usize) -> Option<ToolCall> {
    let v: Value = serde_json::from_str(text.trim()).ok().or_else(|| {
        let mut de = serde_json::Deserializer::from_str(text.trim()).into_iter::<Value>();
        de.next().and_then(Result::ok)
    })?;
    let name = v
        .get("name")
        .or_else(|| v.get("tool"))
        .or_else(|| v.get("function").and_then(|f| f.get("name")))
        .and_then(Value::as_str)?
        .to_string();
    if !known.is_empty() && !known.contains(&name.as_str()) {
        return None;
    }
    let args = v
        .get("arguments")
        .or_else(|| v.get("parameters"))
        .or_else(|| v.get("args"))
        .or_else(|| v.get("function").and_then(|f| f.get("arguments")))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let arguments = match args {
        Value::String(s) => s,
        other => other.to_string(),
    };
    Some(ToolCall {
        id: format!("text_call_{n}"),
        name,
        arguments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        assert_eq!(parse_arguments("").unwrap(), serde_json::json!({}));
        assert_eq!(parse_arguments(r#"{"a":1}"#).unwrap()["a"], 1);
        assert_eq!(parse_arguments(r#""{\"a\":2}""#).unwrap()["a"], 2);
        assert_eq!(parse_arguments(r#"{"a":3} trailing"#).unwrap()["a"], 3);
        assert!(parse_arguments("[1]").is_err());
        assert!(parse_arguments("{broken").is_err());
    }

    #[test]
    fn qwen_style_calls() {
        let text = "Let me look.\n<tool_call>\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.py\"}}\n</tool_call>";
        let (rest, calls) = extract_text_tool_calls(text, &["read_file"]);
        assert_eq!(rest, "Let me look.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].parsed_arguments().unwrap()["path"], "a.py");
    }

    #[test]
    fn fenced_and_unknown() {
        let text = "```json\n{\"name\": \"grep\", \"arguments\": {\"pattern\": \"x\"}}\n```";
        let (_, calls) = extract_text_tool_calls(text, &["grep"]);
        assert_eq!(calls[0].name, "grep");
        let (rest, calls) = extract_text_tool_calls("```json\n{\"name\": \"rm_rf\"}\n```", &["grep"]);
        assert!(calls.is_empty());
        assert!(rest.contains("rm_rf"));
        let (_, calls) = extract_text_tool_calls("plain answer", &["grep"]);
        assert!(calls.is_empty());
    }

    #[test]
    fn thinking() {
        let (c, r) = split_thinking("<think>plan it</think>\nAnswer");
        assert_eq!(c, "Answer");
        assert_eq!(r, "plan it");
        let (c, r) = split_thinking("still thinking</think>Done");
        assert_eq!(c, "Done");
        assert_eq!(r, "still thinking");
    }
}
