//! The Engine.IO v4 / Socket.IO default-namespace text packets used by Injected.
//! Authentication belongs in a Socket.IO CONNECT payload, never in the URL.
use serde_json::{json, Value};

#[derive(Debug, PartialEq)]
pub enum Packet {
    Open { heartbeat_ms: u64 },
    Ping(String),
    Connected,
    Event(String, Value),
    Ack(u64, Value),
    Rejected,
    Closed,
    Ignore,
}

pub fn decode(text: &str) -> Result<Packet, String> {
    if let Some(body) = text.strip_prefix('0') {
        let v: Value = serde_json::from_str(body).map_err(|_| "Invalid Engine.IO handshake")?;
        let interval = v["pingInterval"].as_u64().ok_or("Missing ping interval")?;
        let timeout = v["pingTimeout"].as_u64().ok_or("Missing ping timeout")?;
        return Ok(Packet::Open {
            heartbeat_ms: interval.saturating_add(timeout).clamp(1000, 300_000),
        });
    }
    if let Some(data) = text.strip_prefix('2') {
        return Ok(Packet::Ping(data.to_owned()));
    }
    if text == "1" || text.starts_with("41") {
        return Ok(Packet::Closed);
    }
    if text.starts_with("44") {
        return Ok(Packet::Rejected);
    }
    if text == "40" || text.starts_with("40{") {
        return Ok(Packet::Connected);
    }
    if let Some(body) = text.strip_prefix("42") {
        // Only the default namespace, no binary attachment packets.
        let begin = body.find('[').ok_or("Invalid Socket.IO event")?;
        if !body[..begin].chars().all(|c| c.is_ascii_digit()) {
            return Ok(Packet::Ignore);
        }
        let a: Vec<Value> =
            serde_json::from_str(&body[begin..]).map_err(|_| "Invalid event JSON")?;
        let name = a
            .first()
            .and_then(Value::as_str)
            .ok_or("Missing event name")?;
        return Ok(Packet::Event(
            name.to_string(),
            a.get(1).cloned().unwrap_or(Value::Null),
        ));
    }
    if let Some(body) = text.strip_prefix("43") {
        let begin = body.find('[').ok_or("Invalid Socket.IO acknowledgement")?;
        let id = body[..begin]
            .parse()
            .map_err(|_| "Invalid acknowledgement ID")?;
        let a: Vec<Value> =
            serde_json::from_str(&body[begin..]).map_err(|_| "Invalid acknowledgement JSON")?;
        return Ok(Packet::Ack(id, a.first().cloned().unwrap_or(Value::Null)));
    }
    Ok(Packet::Ignore)
}

pub fn authenticate(token: &str) -> String {
    format!("40{}", json!({"token":token}))
}
pub fn event(name: &str, data: Value, ack: Option<u64>) -> String {
    let id = ack.map(|i| i.to_string()).unwrap_or_default();
    format!("42{}{}", id, json!([name, data]))
}
pub fn no_arg_event(name: &str, ack: u64) -> String {
    format!("42{}{}", ack, json!([name]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_handshake_and_heartbeat() {
        assert_eq!(
            decode(r#"0{"sid":"test","pingInterval":25000,"pingTimeout":20000}"#).unwrap(),
            Packet::Open {
                heartbeat_ms: 45000
            }
        );
        assert_eq!(decode("2probe").unwrap(), Packet::Ping("probe".into()));
        assert!(decode("0{}").is_err());
    }
    #[test]
    fn events_and_acknowledgements() {
        assert_eq!(
            decode(r#"42["irc:event:message",{"message":"hello"}]"#).unwrap(),
            Packet::Event("irc:event:message".into(), json!({"message":"hello"}))
        );
        assert_eq!(
            decode(r#"4312[{"messages":[]}]"#).unwrap(),
            Packet::Ack(12, json!({"messages":[]}))
        );
        assert_eq!(
            decode(r#"44{"message":"Authentication error"}"#).unwrap(),
            Packet::Rejected
        );
        assert_eq!(
            event("history:get", json!({"target":"#rust"}), Some(7)),
            r##"427["history:get",{"target":"#rust"}]"##
        );
        assert!(decode("43oops[]").is_err());
        assert_eq!(decode(r#"42/other,["event",{}]"#).unwrap(), Packet::Ignore);
    }
}
