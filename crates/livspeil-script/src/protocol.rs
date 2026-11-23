// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Wire messages: one JSON object per line (see `docs/script-protocol.md`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};

/// The protocol version this host speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// Error codes carried in `error.code` (stable strings).
pub mod codes {
    /// A line was not valid JSON.
    pub const PARSE_ERROR: &str = "parse_error";
    /// Valid JSON but not a request, response or notification.
    pub const INVALID_REQUEST: &str = "invalid_request";
    /// The method is not known.
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    /// The params do not fit the method.
    pub const INVALID_PARAMS: &str = "invalid_params";
    /// A request other than `hello` came before the handshake.
    pub const HANDSHAKE_REQUIRED: &str = "handshake_required";
    /// The requested protocol version is not supported.
    pub const UNSUPPORTED_PROTOCOL: &str = "unsupported_protocol";
    /// The session lacks the capability the method needs.
    pub const CAPABILITY_DENIED: &str = "capability_denied";
    /// A node path or id does not name a node.
    pub const NOT_FOUND: &str = "not_found";
    /// The command bus rejected a command; the graph is unchanged.
    pub const COMMAND_FAILED: &str = "command_failed";
    /// The request was cancelled.
    pub const CANCELLED: &str = "cancelled";
    /// A handler failed (for example a script's tick handler raised).
    pub const HANDLER_FAILED: &str = "handler_failed";
    /// A host-side failure that is not the caller's fault.
    pub const INTERNAL: &str = "internal";
}

/// The `error` member of a failed response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteError {
    /// One of [`codes`] (or a script-defined code).
    pub code: String,
    /// Human-readable description.
    pub message: String,
    /// Optional structured details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Json>,
}

impl RemoteError {
    /// An error without data.
    #[must_use]
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        RemoteError {
            code: code.to_owned(),
            message: message.into(),
            data: None,
        }
    }

    /// Attach structured details.
    #[must_use]
    pub fn with_data(mut self, data: Json) -> Self {
        self.data = Some(data);
        self
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RemoteError {}

/// A decoded message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Has `id` and `method`; expects a response with the same `id`.
    Request {
        id: Json,
        method: String,
        params: Json,
    },
    /// Has `id` and exactly one of `result` or `error`.
    Response {
        id: Json,
        outcome: Result<Json, RemoteError>,
    },
    /// Has `method` and no `id`; never answered.
    Notification { method: String, params: Json },
}

impl Message {
    /// Classify a JSON value. `Err` holds the reason and the `id`, if one
    /// could be read, so the receiver can answer `invalid_request`.
    ///
    /// # Errors
    /// When the value is not an object of one of the three shapes.
    pub fn decode(value: Json) -> Result<Message, (String, Json)> {
        let Json::Object(mut obj) = value else {
            return Err(("a message must be a JSON object".into(), Json::Null));
        };
        let id = obj.remove("id");
        if let Some(id) = &id {
            if !(id.is_u64() || id.is_i64() || id.is_string()) {
                return Err(("id must be an integer or a string".into(), Json::Null));
            }
        }
        let params = obj.remove("params").unwrap_or(Json::Null);
        match (id, obj.remove("method")) {
            (Some(id), Some(Json::String(method))) => Ok(Message::Request { id, method, params }),
            (None, Some(Json::String(method))) => Ok(Message::Notification { method, params }),
            (_, Some(_)) => Err(("method must be a string".into(), Json::Null)),
            (Some(id), None) => match (obj.remove("result"), obj.remove("error")) {
                (Some(result), None) => Ok(Message::Response {
                    id,
                    outcome: Ok(result),
                }),
                (None, Some(error)) => match serde_json::from_value::<RemoteError>(error) {
                    Ok(err) => Ok(Message::Response {
                        id,
                        outcome: Err(err),
                    }),
                    Err(e) => Err((format!("malformed error object: {e}"), id)),
                },
                _ => Err((
                    "a response needs exactly one of result and error".into(),
                    id,
                )),
            },
            (None, None) => Err(("a message needs a method or an id".into(), Json::Null)),
        }
    }
}

/// `{"id":..,"method":..,"params":..}`.
#[must_use]
pub fn request(id: u64, method: &str, params: Json) -> Json {
    json!({ "id": id, "method": method, "params": params })
}

/// `{"method":..,"params":..}`.
#[must_use]
pub fn notification(method: &str, params: Json) -> Json {
    json!({ "method": method, "params": params })
}

/// A response carrying `outcome`.
#[must_use]
pub fn response(id: Json, outcome: Result<Json, RemoteError>) -> Json {
    let mut obj = Map::new();
    obj.insert("id".into(), id);
    match outcome {
        Ok(result) => {
            obj.insert("result".into(), result);
        }
        Err(err) => {
            obj.insert(
                "error".into(),
                serde_json::to_value(err).unwrap_or(Json::Null),
            );
        }
    }
    Json::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_three_shapes() {
        let req = Message::decode(json!({"id": 1, "method": "graph.nodes"})).unwrap();
        assert!(matches!(req, Message::Request { ref method, .. } if method == "graph.nodes"));
        let note = Message::decode(json!({"method": "log", "params": {"message": "hi"}})).unwrap();
        assert!(matches!(note, Message::Notification { .. }));
        let ok = Message::decode(json!({"id": "h1", "result": null})).unwrap();
        assert_eq!(
            ok,
            Message::Response {
                id: json!("h1"),
                outcome: Ok(Json::Null)
            }
        );
        let err =
            Message::decode(json!({"id": 2, "error": {"code": "x", "message": "y"}})).unwrap();
        assert!(matches!(
            err,
            Message::Response {
                outcome: Err(_),
                ..
            }
        ));
    }

    #[test]
    fn rejects_malformed_messages() {
        assert!(Message::decode(json!([1])).is_err());
        assert!(Message::decode(json!({"id": 1.5, "method": "x"})).is_err());
        assert!(Message::decode(json!({"id": 1})).is_err());
        let (_, id) = Message::decode(json!({"id": 3, "result": 1, "error": {}})).unwrap_err();
        assert_eq!(id, json!(3));
    }

    #[test]
    fn encodes_responses() {
        let ok = response(json!(1), Ok(json!({"a": 1})));
        assert_eq!(ok, json!({"id": 1, "result": {"a": 1}}));
        let err = response(json!(1), Err(RemoteError::new(codes::NOT_FOUND, "no")));
        assert_eq!(
            err,
            json!({"id": 1, "error": {"code": "not_found", "message": "no"}})
        );
    }
}
