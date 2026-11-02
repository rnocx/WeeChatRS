//! Authenticated paste uploads, available only through Injected's backend.
use super::InjectedConfig;
use serde_json::Value;

const MAX_RESPONSE: usize = 64 * 1024;

fn endpoint(config: &InjectedConfig) -> Result<url::Url, String> {
    if !config.use_ssl && !matches!(config.host.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return Err("Injected paste uploads require TLS.".into());
    }
    let mut base = url::Url::parse("https://localhost/api/paste").unwrap();
    base.set_scheme(if config.use_ssl { "https" } else { "http" })
        .unwrap();
    base.set_host(Some(&config.host))
        .map_err(|_| "Invalid Injected host.")?;
    base.set_port(Some(config.port))
        .map_err(|_| "Invalid Injected port.")?;
    Ok(base)
}

fn paste_url(endpoint: &url::Url, response: &Value) -> Result<String, String> {
    let path = response["url"]
        .as_str()
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_whitespace))
        .ok_or("Injected did not return a paste link.")?;
    let url = endpoint
        .join(path)
        .map_err(|_| "Injected returned an invalid paste link.")?;
    if url.origin() != endpoint.origin() || !url.username().is_empty() || url.password().is_some() {
        return Err("Injected returned a paste link outside its server.".into());
    }
    Ok(url.into())
}

pub(super) async fn create(config: &InjectedConfig, content: String) -> Result<String, String> {
    if content.trim().is_empty() {
        return Err("The paste is empty.".into());
    }
    if config.token.trim().is_empty() {
        return Err("Sign in to Injected before sharing a paste.".into());
    }
    let endpoint = endpoint(config)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the paste upload.")?;
    let body = serde_json::to_vec(&serde_json::json!({"content":content}))
        .map_err(|_| "Could not prepare the paste.")?;
    let mut response = client
        .post(endpoint.clone())
        .bearer_auth(config.token.trim())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|_| "Could not upload to Injected. Check your connection and retry.")?;
    if !response.status().is_success() {
        // Never echo response bodies: they could contain pasted text or credentials.
        return Err(match response.status().as_u16() {
            401 => "Your Injected session expired. Sign in again before retrying.",
            403 => "Injected did not allow this account to create a paste.",
            413 => "This paste exceeds Injected's size limit.",
            429 => "Injected's paste rate limit was reached. Please wait before retrying.",
            _ => "Injected could not create the paste. Please retry later.",
        }
        .into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read Injected's paste response.")?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE {
            return Err("Injected's paste response was too large.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Injected returned an invalid paste response.")?;
    paste_url(&endpoint, &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn server(
        status: u16,
        response: &'static str,
    ) -> (InjectedConfig, tokio::task::JoinHandle<Value>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut b = [0];
                socket.read_exact(&mut b).await.unwrap();
                header.push(b[0]);
            }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with("POST /api/paste HTTP/1.1\r\n"));
            assert!(header
                .to_lowercase()
                .contains("authorization: bearer test-token"));
            assert!(header
                .to_lowercase()
                .contains("content-type: application/json"));
            let size: usize = header
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; size];
            socket.read_exact(&mut body).await.unwrap();
            socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
            serde_json::from_slice(&body).unwrap()
        });
        (
            InjectedConfig {
                host: "127.0.0.1".into(),
                port,
                token: "test-token".into(),
                use_ssl: false,
                auto_reconnect: false,
            },
            task,
        )
    }
    #[tokio::test]
    async fn uploads_exact_text_with_session_and_resolves_link() {
        let (config, request) = server(200, r#"{"url":"/paste?id=test-id","lines":4}"#).await;
        let text = "    code\r\n\r\n/quit\n";
        let url = create(&config, text.into()).await.unwrap();
        assert_eq!(
            url,
            format!("http://127.0.0.1:{}/paste?id=test-id", config.port)
        );
        assert_eq!(request.await.unwrap(), serde_json::json!({"content":text}));
    }
    #[tokio::test]
    async fn authentication_and_service_errors_return_no_link_or_response_body() {
        for status in [401, 403, 413, 429, 500, 302] {
            let (config, request) =
                server(status, r#"{"error":"secret-token private pasted content"}"#).await;
            let error = create(&config, "private text".into()).await.unwrap_err();
            assert!(!error.contains("secret-token") && !error.contains("private pasted content"));
            request.await.unwrap();
        }
    }
    #[tokio::test]
    async fn empty_content_and_missing_session_are_rejected_before_network_io() {
        let mut config = InjectedConfig {
            host: "127.0.0.1".into(),
            port: 9,
            token: "dummy".into(),
            use_ssl: false,
            auto_reconnect: false,
        };
        assert!(create(&config, " \n".into())
            .await
            .unwrap_err()
            .contains("empty"));
        config.token.clear();
        assert!(create(&config, "text".into())
            .await
            .unwrap_err()
            .contains("Sign in"));
    }
    #[tokio::test]
    async fn invalid_success_response_is_not_treated_as_a_paste() {
        for body in [
            "not json",
            r#"{"lines":3}"#,
            r#"{"url":"https://other.test/link"}"#,
        ] {
            let (config, request) = server(200, body).await;
            assert!(create(&config, "text".into()).await.is_err());
            request.await.unwrap();
        }
    }

    #[test]
    fn rejects_unsafe_links_and_remote_plain_http() {
        let endpoint = url::Url::parse("https://injected-irc.com/api/paste").unwrap();
        for url in [
            "https://other.test/paste",
            "//other.test/x",
            "javascript:alert(1)",
            "https://user:secret@injected-irc.com/paste",
            "",
        ] {
            assert!(paste_url(&endpoint, &serde_json::json!({"url":url})).is_err());
        }
        let config = InjectedConfig {
            host: "injected-irc.com".into(),
            port: 80,
            token: "test-token".into(),
            use_ssl: false,
            auto_reconnect: false,
        };
        assert!(super::endpoint(&config).is_err());
    }
}
