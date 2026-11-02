//! Anonymous pastes on a PrivateBin server: the text is encrypted here, and the key
//! is only in the link's `#` part, which browsers never send, so the server can't
//! read it. No account or token; the server sees only the encrypted paste.
//! Format: PrivateBin's v2 API (AES-256-GCM, key from PBKDF2-SHA256, raw deflate).

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::aes::Aes256;
use aes_gcm::AesGcm;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use rand::RngCore;
use serde_json::{json, Value};
use std::io::Write;

/// PrivateBin's IV is 16 bytes, not GCM's usual 12.
type Cipher = AesGcm<Aes256, aes_gcm::aead::consts::U16>;

const ITERATIONS: u32 = 100_000;
const MAX_RESPONSE: usize = 64 * 1024;

pub const DEFAULT_SERVER: &str = "https://0bin.ch/";

/// PrivateBin's expiry choices, as (API value, label).
pub const EXPIRY: &[(&str, &str)] = &[
    ("5min", "5 minutes"),
    ("10min", "10 minutes"),
    ("1hour", "1 hour"),
    ("1day", "1 day"),
    ("1week", "1 week"),
    ("1month", "1 month"),
    ("1year", "1 year"),
    ("never", "Never"),
];

#[derive(Clone, Debug)]
pub struct Options {
    pub server: String,
    pub expire: String,
    pub burn_after_reading: bool,
}

/// A created paste: the link to share, and the one that deletes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Created {
    pub url: String,
    pub delete_url: String,
}

/// The server's address as a base for links, without a trailing slash. Only https,
/// except for a server on this computer.
pub fn normalize_server(input: &str) -> Result<url::Url, String> {
    let text = input.trim();
    if text.is_empty() {
        return Err("No PrivateBin server set".to_string());
    }
    let mut url = url::Url::parse(text).map_err(|_| format!("Not a server address: {text}"))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err("The PrivateBin server must use https://".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The server address can't hold a user name or password".to_string());
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// The paste's encrypted form for the server: `(adata, ct)`, from its key, IV and salt.
fn encrypt(text: &str, burn: bool, key: &[u8; 32], iv: &[u8; 16], salt: &[u8; 8]) -> Result<(Value, String), String> {
    let spec = json!([B64.encode(iv), B64.encode(salt), ITERATIONS, 256, 128, "aes", "gcm", "zlib"]);
    let adata = json!([spec, "plaintext", 0, u8::from(burn)]);
    let mut derived = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(key, salt, ITERATIONS, &mut derived);

    let message = serde_json::to_vec(&json!({ "paste": text })).map_err(|e| e.to_string())?;
    let mut deflate = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    deflate.write_all(&message).map_err(|e| e.to_string())?;
    let compressed = deflate.finish().map_err(|e| e.to_string())?;

    // The additional data is adata as JSON, exactly as sent.
    let aad = serde_json::to_string(&adata).map_err(|e| e.to_string())?;
    let cipher = Cipher::new_from_slice(&derived).map_err(|e| e.to_string())?;
    let sealed = cipher
        .encrypt(iv.into(), Payload { msg: &compressed, aad: aad.as_bytes() })
        .map_err(|_| "Couldn't encrypt the paste".to_string())?;
    Ok((adata, B64.encode(sealed)))
}

/// The links of a paste the server stored as `id`.
fn links(server: &url::Url, id: &str, delete_token: &str, key: &[u8; 32]) -> Created {
    Created {
        url: format!("{server}?{id}#{}", bs58::encode(key).into_string()),
        delete_url: format!("{server}?pasteid={id}&deletetoken={delete_token}"),
    }
}

/// An error with its causes, which reqwest's own message leaves out.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

/// Encrypts `text` and stores it on the server.
pub async fn create(options: &Options, text: String) -> Result<Created, String> {
    if text.trim().is_empty() {
        return Err("The paste is empty".to_string());
    }
    let server = normalize_server(&options.server)?;
    let (mut key, mut iv, mut salt) = ([0u8; 32], [0u8; 16], [0u8; 8]);
    let mut rng = rand::rngs::OsRng;
    rng.fill_bytes(&mut key);
    rng.fill_bytes(&mut iv);
    rng.fill_bytes(&mut salt);
    let burn = options.burn_after_reading;
    // The key derivation takes a moment; keep it off the async workers.
    let (adata, ct) = tokio::task::spawn_blocking(move || encrypt(&text, burn, &key, &iv, &salt))
        .await
        .map_err(|e| e.to_string())??;
    let body = json!({ "v": 2, "adata": adata, "ct": ct, "meta": { "expire": options.expire } }).to_string();
    let size = crate::ui::fileshare::human_size(body.len() as u64);

    // No cookies, no redirects and no client name: nothing that tells who sent it.
    // rustls, because macOS's own TLS stops at 1.2 and some servers (privatebin.net)
    // only accept 1.3.
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;
    let mut response = client
        .post(server.clone())
        .header("X-Requested-With", "JSONHttpRequest")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Couldn't reach the PrivateBin server: {}", error_chain(&e.without_url())))?;
    if response.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
        return Err(format!("The paste is too large for this server ({size} once encrypted). Upload it as a file instead, or use a server that takes larger pastes."));
    }
    if !response.status().is_success() {
        return Err(format!("The PrivateBin server answered {}", response.status()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "Couldn't read the server's answer")? {
        if bytes.len() + chunk.len() > MAX_RESPONSE {
            return Err("The server's answer was too large".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    let reply: Value = serde_json::from_slice(&bytes).map_err(|_| "That isn't a PrivateBin server".to_string())?;
    if reply["status"].as_i64() != Some(0) {
        let message = reply["message"].as_str().unwrap_or("it refused the paste");
        return Err(format!("PrivateBin: {}", message.chars().take(200).collect::<String>()));
    }
    let safe = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    let id = reply["id"].as_str().filter(|s| safe(s)).ok_or("The server returned no paste id")?;
    let token = reply["deletetoken"].as_str().filter(|s| safe(s)).ok_or("The server returned no delete token")?;
    Ok(links(&server, id, token, &key))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decrypts as PrivateBin's viewer does, to check the format end to end.
    fn decrypt(adata: &Value, ct: &str, key: &[u8; 32]) -> String {
        let spec = &adata[0];
        let iv = B64.decode(spec[0].as_str().unwrap()).unwrap();
        let salt = B64.decode(spec[1].as_str().unwrap()).unwrap();
        let mut derived = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(key, &salt, spec[2].as_u64().unwrap() as u32, &mut derived);
        let aad = serde_json::to_string(adata).unwrap();
        let plain = Cipher::new_from_slice(&derived).unwrap()
            .decrypt(iv.as_slice().into(), Payload { msg: &B64.decode(ct).unwrap(), aad: aad.as_bytes() })
            .unwrap();
        let mut json = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(plain.as_slice()), &mut json).unwrap();
        serde_json::from_slice::<Value>(&json).unwrap()["paste"].as_str().unwrap().to_string()
    }

    #[test]
    fn encrypts_in_privatebins_format() {
        let (key, iv, salt) = ([7u8; 32], [1u8; 16], [2u8; 8]);
        let text = "fn main() {\n    println!(\"hi\");\n}\n\n/quit ✓";
        let (adata, ct) = encrypt(text, true, &key, &iv, &salt).unwrap();
        assert_eq!(adata[0][2], 100_000);
        assert_eq!(adata[0][7], "zlib");
        assert_eq!(adata[1], "plaintext");
        assert_eq!(adata[3], 1);
        assert_eq!(decrypt(&adata, &ct, &key), text);
    }

    #[test]
    fn links_carry_the_key_only_after_the_hash() {
        let server = normalize_server("https://paste.example.org/bin").unwrap();
        let made = links(&server, "abc123", "tok", &[0u8; 32]);
        assert_eq!(made.url, format!("https://paste.example.org/bin/?abc123#{}", "1".repeat(32)));
        assert_eq!(made.delete_url, "https://paste.example.org/bin/?pasteid=abc123&deletetoken=tok");
    }

    #[test]
    fn servers_must_use_https_except_locally() {
        assert_eq!(normalize_server(" https://privatebin.net ").unwrap().as_str(), "https://privatebin.net/");
        assert_eq!(normalize_server("https://x.org/p?x=1#y").unwrap().as_str(), "https://x.org/p/");
        assert!(normalize_server("http://privatebin.net").is_err());
        assert!(normalize_server("http://localhost:8080").is_ok());
        assert!(normalize_server("https://me:pw@x.org").is_err());
        assert!(normalize_server("").is_err());
    }

    /// Uploads to a PrivateBin server named by PRIVATEBIN_TEST_SERVER and prints the link.
    #[tokio::test]
    #[ignore]
    async fn live_upload() {
        let server = std::env::var("PRIVATEBIN_TEST_SERVER").expect("PRIVATEBIN_TEST_SERVER");
        let options = Options { server, expire: "5min".into(), burn_after_reading: false };
        // PRIVATEBIN_TEST_KB makes a paste of about that many kilobytes.
        let text = match std::env::var("PRIVATEBIN_TEST_KB").ok().and_then(|k| k.parse::<usize>().ok()) {
            // Random text barely compresses, so the request is about 1.4 × this size.
            Some(kb) => {
                let mut bytes = vec![0u8; kb * 1024 * 3 / 4];
                rand::rngs::OsRng.fill_bytes(&mut bytes);
                B64.encode(bytes)
            }
            None => "WeeChatRS test\n  indented\n\n✓".to_string(),
        };
        let made = create(&options, text).await.unwrap();
        println!("URL {}\nDELETE {}", made.url, made.delete_url);
    }
}
