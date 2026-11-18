//! Wire protocol for the herdr socket: newline-delimited JSON.
//!
//! Requests: `{"id","method","params"}`. Responses: `{"id","result"}` or
//! `{"id","error":{"code","message"}}`. After an `events.subscribe` ack, pushed
//! lines look like `{"event": <kind>, "data": {...}}` on the same connection.
//! `kind` is snake_case for lifecycle events (`pane_closed`) but dotted for
//! subscription events (`pane.agent_status_changed`); consumers should normalise.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("malformed message: {0}")]
    Malformed(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub params: Value,
}

impl Request {
    /// Serialise to one newline-terminated JSON line.
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("Request always serialises");
        s.push('\n');
        s
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HerdrEvent {
    pub kind: String,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response { id: String, result: Value },
    Error { id: String, code: String, message: String },
    Event(HerdrEvent),
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn parse_line(line: &str) -> Result<Incoming, ProtocolError> {
    let v: Value = serde_json::from_str(line.trim())?;
    if let (Some(kind), Some(data)) = (v.get("event").and_then(Value::as_str), v.get("data")) {
        return Ok(Incoming::Event(HerdrEvent { kind: kind.to_string(), data: data.clone() }));
    }
    let id = str_field(&v, "id");
    if let Some(err) = v.get("error") {
        return Ok(Incoming::Error {
            id,
            code: str_field(err, "code"),
            message: str_field(err, "message"),
        });
    }
    if let Some(result) = v.get("result") {
        return Ok(Incoming::Response { id, result: result.clone() });
    }
    Err(ProtocolError::Malformed(line.trim().chars().take(120).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_response_error_and_event_lines() {
        let r = parse_line(r#"{"id":"1","result":{"type":"ok"},"extra":1}"#).unwrap();
        assert_eq!(r, Incoming::Response { id: "1".into(), result: json!({"type":"ok"}) });

        let e = parse_line(r#"{"id":"2","error":{"code":"pane_not_found","message":"nope","x":1}}"#).unwrap();
        assert_eq!(
            e,
            Incoming::Error { id: "2".into(), code: "pane_not_found".into(), message: "nope".into() }
        );

        let ev = parse_line(r#"{"event":"pane_closed","data":{"pane_id":"w1:p1"},"extra":true}"#).unwrap();
        assert_eq!(
            ev,
            Incoming::Event(HerdrEvent { kind: "pane_closed".into(), data: json!({"pane_id":"w1:p1"}) })
        );

        assert!(parse_line("not json").is_err());
        assert!(parse_line("{}").is_err());
    }

    #[test]
    fn request_to_line_is_newline_terminated_json() {
        let req = Request { id: "a".into(), method: "ping".into(), params: json!({}) };
        let line = req.to_line();
        assert!(line.ends_with('\n'));
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v, json!({"id":"a","method":"ping","params":{}}));
    }

    #[test]
    fn captured_event_fixture_parses() {
        let text = include_str!("../../tests/fixtures/events.ndjson");
        let parsed: Vec<_> = text.lines().map(|l| parse_line(l).unwrap()).collect();
        assert!(matches!(parsed[0], Incoming::Response { .. }));
        assert!(parsed[1..].iter().all(|i| matches!(i, Incoming::Event(_))));
    }
}
