//! Model layer.
//!
//! The agent talks to models only through [`ModelProvider`]. Implementations:
//!
//! * [`openai::OpenAiCompatProvider`]: any OpenAI-compatible chat completions
//!   endpoint. This covers Kara's managed llama.cpp server, Ollama,
//!   LM Studio, vLLM and others.
//! * [`scripted::ScriptedProvider`]: deterministic replay for tests, CI and
//!   offline evaluation of the agent loop.
//!
//! Nothing here is specific to one model family.

pub mod hardware;
pub mod openai;
pub mod recommend;
pub mod registry;
pub mod scripted;
pub mod toolparse;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use kara_protocol::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments string as produced by the model.
    pub arguments: String,
}

impl ToolCall {
    /// Parse arguments leniently: accepts objects, JSON-encoded strings and
    /// trailing garbage after a complete object.
    pub fn parsed_arguments(&self) -> Result<Value, String> {
        toolparse::parse_arguments(&self.arguments)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(c: impl Into<String>) -> Self {
        Self::new(Role::System, c)
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self::new(Role::User, c)
    }
    pub fn assistant(c: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls,
            ..Self::new(Role::Assistant, c)
        }
    }
    pub fn tool(call_id: impl Into<String>, name: impl Into<String>, c: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(call_id.into()),
            name: Some(name.into()),
            ..Self::new(Role::Tool, c)
        }
    }
    fn new(role: Role, c: impl Into<String>) -> Self {
        Self {
            role,
            content: c.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    /// Rough token estimate (chars / 3.5) for context budgeting when the
    /// runtime does not report usage.
    pub fn estimated_tokens(&self) -> usize {
        let chars = self.content.len()
            + self
                .tool_calls
                .iter()
                .map(|t| t.name.len() + t.arguments.len())
                .sum::<usize>();
        chars * 2 / 7 + 4
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Text(String),
    Reasoning(String),
    ToolCall(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatResponse {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderInfo {
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub local: bool,
    pub context_length: Option<u32>,
}

pub type EventSink<'a> = &'a (dyn Fn(StreamEvent) + Send + Sync);

#[async_trait::async_trait]
pub trait ModelProvider: Send + Sync {
    fn info(&self) -> ProviderInfo;
    async fn chat(
        &self,
        request: ChatRequest,
        on_event: EventSink<'_>,
        cancel: &CancellationToken,
    ) -> anyhow::Result<ChatResponse>;
}
