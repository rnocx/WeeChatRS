//! What the sign-in window hands the app: the session from Injected's official page,
//! and the checks both sides make on it. Also compiled into the Linux sign-in helper.
use serde::{Deserialize, Serialize};

pub const ORIGIN: &str = "https://injected-irc.com";
pub const MAX_REPLY: usize = 32 * 1024;

#[derive(Deserialize, Serialize)]
pub struct SessionReply {
    pub token: String,
}
pub fn official_url(url: &str) -> bool {
    url::Url::parse(url)
        .map(|u| {
            u.scheme() == "https"
                && u.host_str() == Some("injected-irc.com")
                && u.port_or_known_default() == Some(443)
                && u.username().is_empty()
                && u.password().is_none()
        })
        .unwrap_or(false)
}
pub fn parse_reply(origin: &str, body: &str) -> Option<SessionReply> {
    if !official_url(origin) || body.len() > MAX_REPLY {
        return None;
    }
    let reply: SessionReply = serde_json::from_str(body).ok()?;
    if reply.token.is_empty()
        || reply.token.len() > 16384
        || reply.token.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_accepts_only_the_official_https_origin() {
        assert!(parse_reply(
            "https://injected-irc.com/#login",
            r#"{"token":"test-token"}"#
        )
        .is_some());
        for origin in [
            "https://injected-irc.com.evil.test",
            "http://injected-irc.com",
            "https://challenges.cloudflare.com",
            "https://injected-irc.com:444",
            "file:///login",
        ] {
            assert!(parse_reply(origin, r#"{"token":"test-token"}"#).is_none());
        }
        assert!(parse_reply(ORIGIN, r#"{"token":""}"#).is_none());
        assert!(parse_reply(ORIGIN, r#"{"token":"bad\ntoken"}"#).is_none());
        assert!(parse_reply(ORIGIN, &"x".repeat(MAX_REPLY + 1)).is_none());
    }
}
