//! Minimal JSON-RPC 2.0 framing used between `kara serve --stdio` and editor
//! clients. Messages are newline-delimited JSON objects (one message per line,
//! UTF-8). Both peers may send requests: the client drives sessions, and the
//! server asks the client for permission decisions.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub mod methods {
    // client -> server requests
    pub const INITIALIZE: &str = "initialize";
    pub const SHUTDOWN: &str = "shutdown";
    pub const PROMPT: &str = "session/prompt";
    pub const CANCEL: &str = "session/cancel";
    pub const COMMAND: &str = "session/command";
    pub const STATUS: &str = "session/status";
    pub const CHANGES: &str = "session/changes";
    pub const UNDO: &str = "session/undo";
    pub const NEW_SESSION: &str = "session/new";
    pub const LIST_SESSIONS: &str = "session/list";
    pub const RESUME_SESSION: &str = "session/resume";
    pub const DOCTOR: &str = "doctor";
    pub const MODELS: &str = "models/list";
    pub const SELECT_MODEL: &str = "models/select";

    // server -> client notifications
    pub const EVENT: &str = "event";
    pub const LOG: &str = "log";

    // server -> client requests
    pub const PERMISSION: &str = "permission/request";
}

pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// A prompt was sent while another turn is still running.
    pub const BUSY: i64 = -32001;
    /// No model is configured or the runtime failed to start.
    pub const MODEL_UNAVAILABLE: i64 = -32002;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// A decoded JSON-RPC message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: Value,
        result: Result<Value, RpcError>,
    },
}

impl Message {
    pub fn request(id: impl Into<Value>, method: &str, params: Value) -> Self {
        Message::Request {
            id: id.into(),
            method: method.to_string(),
            params,
        }
    }

    pub fn notification(method: &str, params: Value) -> Self {
        Message::Notification {
            method: method.to_string(),
            params,
        }
    }

    pub fn response(id: Value, result: Result<Value, RpcError>) -> Self {
        Message::Response { id, result }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Message::Request { id, method, params } => {
                json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
            }
            Message::Notification { method, params } => {
                json!({"jsonrpc": "2.0", "method": method, "params": params})
            }
            Message::Response { id, result } => match result {
                Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
                Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
            },
        }
    }

    /// Encode as a single line (no embedded newlines; serde_json escapes them).
    pub fn encode_line(&self) -> String {
        let mut s = self.to_value().to_string();
        s.push('\n');
        s
    }

    pub fn decode(line: &str) -> Result<Message, RpcError> {
        let v: Value = serde_json::from_str(line.trim())
            .map_err(|e| RpcError::new(codes::PARSE_ERROR, format!("parse error: {e}")))?;
        Self::from_value(v)
    }

    pub fn from_value(v: Value) -> Result<Message, RpcError> {
        let obj = v
            .as_object()
            .ok_or_else(|| RpcError::new(codes::INVALID_REQUEST, "message must be an object"))?;
        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(RpcError::new(
                codes::INVALID_REQUEST,
                "missing jsonrpc: \"2.0\"",
            ));
        }
        let id = obj.get("id").cloned();
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        match (obj.get("method").and_then(Value::as_str), id) {
            (Some(method), Some(id)) => Ok(Message::Request {
                id,
                method: method.to_string(),
                params,
            }),
            (Some(method), None) => Ok(Message::Notification {
                method: method.to_string(),
                params,
            }),
            (None, Some(id)) => {
                if let Some(err) = obj.get("error") {
                    let err: RpcError = serde_json::from_value(err.clone()).map_err(|e| {
                        RpcError::new(codes::INVALID_REQUEST, format!("bad error object: {e}"))
                    })?;
                    Ok(Message::Response {
                        id,
                        result: Err(err),
                    })
                } else {
                    Ok(Message::Response {
                        id,
                        result: Ok(obj.get("result").cloned().unwrap_or(Value::Null)),
                    })
                }
            }
            (None, None) => Err(RpcError::new(
                codes::INVALID_REQUEST,
                "message has neither method nor id",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_kinds() {
        let msgs = vec![
            Message::request(1, methods::PROMPT, json!({"text": "fix\nit"})),
            Message::notification(methods::EVENT, json!({"type": "phase", "phase": "edit"})),
            Message::response(json!(1), Ok(json!({"ok": true}))),
            Message::response(json!("a"), Err(RpcError::new(codes::BUSY, "busy"))),
        ];
        for m in msgs {
            let line = m.encode_line();
            assert_eq!(line.matches('\n').count(), 1, "one line per message");
            assert_eq!(Message::decode(&line).unwrap(), m);
        }
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(
            Message::decode("not json").unwrap_err().code,
            codes::PARSE_ERROR
        );
        assert_eq!(
            Message::decode(r#"{"id":1,"method":"x"}"#)
                .unwrap_err()
                .code,
            codes::INVALID_REQUEST
        );
        assert_eq!(
            Message::decode(r#"{"jsonrpc":"2.0"}"#).unwrap_err().code,
            codes::INVALID_REQUEST
        );
    }
}
