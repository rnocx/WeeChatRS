use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

/// Default Jirafeau server (the "selfhost" setting overrides it).
pub const DEFAULT_JIRAFEAU_URL: &str = "https://files.interdo.me";
const LITTERBOX_URL: &str = "https://litterbox.catbox.moe/resources/internals/api.php";

/// Valid Jirafeau expiry keywords. Anything else falls back to "day". A server may
/// disable some of these; it then rejects the upload (see [`jirafeau_error`]).
const JIRAFEAU_VALID_TIMES: &[&str] = &[
    "minute", "hour", "day", "week", "fortnight", "month", "quarter", "year", "none",
];

/// Valid Litterbox expiry keywords. Anything else falls back to "24h".
pub const LITTERBOX_TIMES: &[(&str, &str)] = &[
    ("1h",  "1 hour"),
    ("12h", "12 hours"),
    ("24h", "24 hours"),
    ("72h", "72 hours"),
];

/// Largest chunk sent per request, even when the server would accept more, so progress
/// stays smooth and a failed request doesn't waste much.
const MAX_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
/// Granularity of progress updates within a request body.
const STREAM_PIECE: usize = 64 * 1024;

#[derive(Serialize, Deserialize, Clone, PartialEq, Default, Debug)]
pub enum FileShareService {
    #[default]
    Jirafeau,
    Litterbox,
}

/// Jirafeau server and credentials for an upload.
#[derive(Clone, Debug, Default)]
pub struct JirafeauOptions {
    /// Base URL of the instance, e.g. `https://files.example.com` (normalised on use).
    pub base_url: String,
    /// Needed when the server restricts who may upload. Empty = none.
    pub upload_password: String,
    /// Protects the uploaded file; recipients must enter it. Empty = none.
    pub download_password: String,
}

/// Upload progress shared between the upload task and the UI.
#[derive(Default)]
pub struct UploadProgress {
    sent: AtomicU64,
    total: AtomicU64,
    cancelled: AtomicBool,
    file_name: Mutex<String>,
}

impl UploadProgress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn sent(&self) -> u64 { self.sent.load(Ordering::Relaxed) }
    pub fn total(&self) -> u64 { self.total.load(Ordering::Relaxed) }
    pub fn fraction(&self) -> f32 {
        let total = self.total();
        if total == 0 { 0.0 } else { (self.sent() as f64 / total as f64).min(1.0) as f32 }
    }
    pub fn file_name(&self) -> String { self.file_name.lock().map(|n| n.clone()).unwrap_or_default() }
    pub fn cancel(&self) { self.cancelled.store(true, Ordering::Relaxed); }
    fn is_cancelled(&self) -> bool { self.cancelled.load(Ordering::Relaxed) }
    fn start(&self, file_name: &str, total: u64) {
        if let Ok(mut n) = self.file_name.lock() { *n = file_name.to_string(); }
        self.total.store(total, Ordering::Relaxed);
        self.sent.store(0, Ordering::Relaxed);
    }
}

/// Server details shown by Settings → File sharing → Test server.
pub struct JirafeauInfo {
    pub version: String,
    /// Maximum file size in MB; 0 = unlimited.
    pub max_file_mb: u64,
    /// Largest single request the server accepts (PHP limits), in bytes.
    pub request_limit: u64,
}

/// Upload `path` and return the public download URL on success.
pub async fn upload(
    path: PathBuf,
    service: &FileShareService,
    duration: &str,
    jirafeau: &JirafeauOptions,
    progress: Arc<UploadProgress>,
) -> Result<String, String> {
    match service {
        FileShareService::Jirafeau => upload_jirafeau(&path, duration, jirafeau, &progress).await,
        FileShareService::Litterbox => upload_litterbox(&path, duration, &progress).await,
    }
}

/// Normalise a user-entered server address: trims whitespace, a trailing `/` and a
/// trailing `/script.php` or `/f.php`; requires http(s).
pub fn normalize_base_url(input: &str) -> Result<String, String> {
    let mut url = input.trim().trim_end_matches('/').to_string();
    for page in ["/script.php", "/f.php", "/index.php"] {
        if let Some(stripped) = url.strip_suffix(page) {
            url = stripped.trim_end_matches('/').to_string();
        }
    }
    if url.is_empty() {
        return Err("No Jirafeau server set".to_string());
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("Server address must start with https:// (got \"{url}\")"));
    }
    Ok(url)
}

/// Ask a Jirafeau server for its version and size limits.
pub async fn jirafeau_server_info(base_url: &str) -> Result<JirafeauInfo, String> {
    let base = normalize_base_url(base_url)?;
    let client = http_client(std::time::Duration::from_secs(15))?;
    let get = |query: &'static str| {
        let client = client.clone();
        let url = format!("{base}/script.php?{query}");
        async move {
            let resp = client.get(&url).send().await.map_err(|e| format!("Cannot reach server: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Server returned {} for {url}", resp.status()));
            }
            resp.text().await.map(|t| t.trim().to_string()).map_err(|e| e.to_string())
        }
    };
    let version = get("get_version").await?;
    if version.is_empty() || version.contains('<') {
        return Err("That doesn't look like a Jirafeau server (no version from script.php)".to_string());
    }
    let max_file_mb = get("get_maximal_upload_size").await?.parse().unwrap_or(0);
    let request_limit = get("get_capacity").await?.parse().unwrap_or(0);
    Ok(JirafeauInfo { version, max_file_mb, request_limit })
}

async fn upload_jirafeau(
    path: &Path,
    duration: &str,
    opts: &JirafeauOptions,
    progress: &Arc<UploadProgress>,
) -> Result<String, String> {
    let base = normalize_base_url(&opts.base_url)?;
    let filename = file_name_of(path);
    let mime = mime_for(&filename);
    let total = tokio::fs::metadata(path).await.map_err(|e| format!("Cannot read file: {e}"))?.len();
    if total == 0 {
        return Err("The file is empty".to_string());
    }
    progress.start(&filename, total);

    let info = jirafeau_server_info(&base).await?;
    if info.max_file_mb > 0 && total > info.max_file_mb * 1024 * 1024 {
        return Err(format!(
            "{filename} is {}; this server accepts up to {} MB",
            human_size(total),
            info.max_file_mb
        ));
    }
    // Leave room for multipart overhead under the server's request limit.
    let chunk_size = match info.request_limit {
        0 => MAX_CHUNK_BYTES,
        limit => (limit.saturating_sub(64 * 1024)).clamp(64 * 1024, MAX_CHUNK_BYTES),
    } as usize;

    let client = http_client(std::time::Duration::from_secs(300))?;
    let script = format!("{base}/script.php");
    let time = if JIRAFEAU_VALID_TIMES.contains(&duration) { duration } else { "day" };

    // 1. Start the transfer.
    let mut form = reqwest::multipart::Form::new()
        .text("filename", filename.clone())
        .text("type", mime.to_string())
        .text("time", time.to_string());
    if !opts.upload_password.is_empty() {
        form = form.text("upload_password", opts.upload_password.clone());
    }
    if !opts.download_password.is_empty() {
        form = form.text("key", opts.download_password.clone());
    }
    let init = jirafeau_post(&client, &format!("{script}?init_async"), form).await?;
    let mut lines = init.lines();
    let (Some(reference), Some(mut code)) = (lines.next().map(str::to_string), lines.next().map(str::to_string)) else {
        return Err(format!("Unexpected reply from server: {init}"));
    };

    // 2. Send the file in chunks; each reply is the code for the next request.
    let mut file = tokio::fs::File::open(path).await.map_err(|e| format!("Cannot read file: {e}"))?;
    let mut buf = vec![0u8; chunk_size];
    loop {
        let n = read_full(&mut file, &mut buf).await.map_err(|e| format!("Cannot read file: {e}"))?;
        if n == 0 {
            break;
        }
        if progress.is_cancelled() {
            return Err(CANCELLED.to_string());
        }
        let part = reqwest::multipart::Part::stream_with_length(counted_body(buf[..n].to_vec(), progress.clone()), n as u64)
            .file_name(filename.clone())
            .mime_str("application/octet-stream")
            .map_err(|e| format!("MIME error: {e}"))?;
        let form = reqwest::multipart::Form::new()
            .text("ref", reference.clone())
            .text("code", code.clone())
            .part("data", part);
        code = jirafeau_post(&client, &format!("{script}?push_async"), form).await?.trim().to_string();
    }

    // 3. Finish: reply is link code, delete code, and the decryption key (if encrypted).
    let form = reqwest::multipart::Form::new().text("ref", reference).text("code", code);
    let end = jirafeau_post(&client, &format!("{script}?end_async"), form).await?;
    let mut lines = end.lines().map(str::trim);
    let hash = lines.next().unwrap_or_default();
    let _delete_code = lines.next();
    let crypt_key = lines.next().unwrap_or_default();
    if hash.is_empty() {
        return Err(format!("Unexpected reply from server: {end}"));
    }
    Ok(jirafeau_link(&base, hash, crypt_key, &filename, mime, !opts.download_password.is_empty()))
}

/// Download link for an upload. Previewable files get `&p=1` (shown inline instead of on
/// a download page) and the filename as a `#fragment` so clients recognise images —
/// unless a download password protects the file, which a preview can't get past.
fn jirafeau_link(base: &str, hash: &str, crypt_key: &str, filename: &str, mime: &str, password_protected: bool) -> String {
    let mut url = format!("{base}/f.php?h={hash}");
    if !crypt_key.is_empty() {
        url.push_str("&k=");
        url.push_str(crypt_key); // already URL-encoded by the server
    }
    let previewable = ["image/", "video/", "audio/", "text/"].iter().any(|t| mime.starts_with(t)) || mime == "application/pdf";
    if previewable && !password_protected {
        url.push_str("&p=1#");
        url.push_str(&encode_fragment(filename));
    }
    url
}

async fn jirafeau_post(client: &reqwest::Client, url: &str, form: reqwest::multipart::Form) -> Result<String, String> {
    let resp = client
        .post(url)
        // Some PHP/nginx setups mis-parse multipart without this (matches curl --http1.0).
        .header("Connection", "close")
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Upload failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("Failed to read response: {e}"))?;
    if !status.is_success() {
        return Err(format!("Server returned {status}"));
    }
    if body.trim_start().starts_with("Error") {
        return Err(jirafeau_error(body.trim()));
    }
    Ok(body)
}

/// Turn a Jirafeau `Error N…` reply into a readable message.
fn jirafeau_error(reply: &str) -> String {
    let code: Option<u32> = reply
        .strip_prefix("Error")
        .map(str::trim_start)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse().ok());
    let msg = match code {
        Some(2) | Some(19) => "This server requires an upload password (Settings → File sharing)",
        Some(3) | Some(20) => "The upload password was rejected",
        Some(4) | Some(22) => "This server doesn't allow that expiry time; choose another in Settings → File sharing",
        Some(5) => "The file is larger than this server allows",
        Some(13) => "This server requires a download password (Settings → File sharing)",
        Some(14) => "The download password doesn't meet this server's password rules",
        Some(26) => "One-time downloads are disabled on this server",
        _ => return format!("Server error: {reply}"),
    };
    msg.to_string()
}

async fn upload_litterbox(path: &Path, duration: &str, progress: &Arc<UploadProgress>) -> Result<String, String> {
    let filename = file_name_of(path);
    let bytes = tokio::fs::read(path).await.map_err(|e| format!("Cannot read file: {e}"))?;
    progress.start(&filename, bytes.len() as u64);
    let time = if LITTERBOX_TIMES.iter().any(|(v, _)| *v == duration) { duration } else { "24h" };
    let len = bytes.len() as u64;

    let file_part = reqwest::multipart::Part::stream_with_length(counted_body(bytes, progress.clone()), len)
        .file_name(filename.clone())
        .mime_str(mime_for(&filename))
        .map_err(|e| format!("MIME error: {e}"))?;
    let form = reqwest::multipart::Form::new()
        .text("reqtype", "fileupload")
        .text("time", time.to_string())
        .part("fileToUpload", file_part);

    let resp = http_client(std::time::Duration::from_secs(600))?
        .post(LITTERBOX_URL)
        .multipart(form)
        .send()
        .await
        .map_err(|e| if progress.is_cancelled() { CANCELLED.to_string() } else { format!("Upload failed: {e}") })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let body = body.trim();
        return Err(if body.is_empty() {
            format!("Server returned {status}")
        } else {
            format!("Server returned {status}: {body}")
        });
    }
    let url = resp.text().await.map_err(|e| format!("Failed to read response: {e}"))?.trim().to_string();
    if url.is_empty() || url.starts_with("err") {
        return Err(format!("Server error: {url}"));
    }
    Ok(url)
}

/// Message used when the user cancels an upload (the UI shows no error for it).
pub const CANCELLED: &str = "Upload cancelled";

fn http_client(timeout: std::time::Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .http1_only()
        .user_agent(concat!("WeeChatRS/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(timeout)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))
}

/// A request body that reports progress as it's sent, and aborts if cancelled.
fn counted_body(data: Vec<u8>, progress: Arc<UploadProgress>) -> reqwest::Body {
    let pieces: Vec<Vec<u8>> = data.chunks(STREAM_PIECE).map(<[u8]>::to_vec).collect();
    let stream = futures_util::stream::iter(pieces.into_iter().map(move |piece| {
        if progress.is_cancelled() {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, CANCELLED));
        }
        progress.sent.fetch_add(piece.len() as u64, Ordering::Relaxed);
        Ok(piece)
    }));
    reqwest::Body::wrap_stream(stream)
}

/// Fill `buf` as far as the file allows; returns bytes read (0 at end of file).
async fn read_full(file: &mut tokio::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]).await? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".to_string())
}

/// Percent-encode a filename for use in a URL fragment.
fn encode_fragment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    match bytes as f64 {
        b if b >= MB * 1024.0 => format!("{:.1} GB", b / (MB * 1024.0)),
        b if b >= MB => format!("{:.1} MB", b / MB),
        b if b >= 1024.0 => format!("{:.0} KB", b / 1024.0),
        b => format!("{b:.0} B"),
    }
}

fn mime_for(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png"              => "image/png",
        "jpg" | "jpeg"     => "image/jpeg",
        "gif"              => "image/gif",
        "webp"             => "image/webp",
        "svg"              => "image/svg+xml",
        "mp4"              => "video/mp4",
        "webm"             => "video/webm",
        "mp3"              => "audio/mpeg",
        "ogg"              => "audio/ogg",
        "pdf"              => "application/pdf",
        "zip"              => "application/zip",
        "txt" | "log"
            | "md"         => "text/plain",
        _                  => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_server_addresses() {
        assert_eq!(normalize_base_url(" https://files.example.com/ ").unwrap(), "https://files.example.com");
        assert_eq!(normalize_base_url("https://x.org/jirafeau/script.php").unwrap(), "https://x.org/jirafeau");
        assert_eq!(normalize_base_url("https://x.org/f.php").unwrap(), "https://x.org");
        assert!(normalize_base_url("files.example.com").is_err());
        assert!(normalize_base_url("   ").is_err());
    }

    #[test]
    fn links_include_key_preview_and_filename() {
        assert_eq!(
            jirafeau_link("https://f.io", "abc", "", "Screen shot.png", "image/png", false),
            "https://f.io/f.php?h=abc&p=1#Screen%20shot.png"
        );
        assert_eq!(
            jirafeau_link("https://f.io", "abc", "K%2Bey", "a.zip", "application/zip", false),
            "https://f.io/f.php?h=abc&k=K%2Bey"
        );
        // A password-protected file can't be previewed, so no preview hint.
        assert_eq!(
            jirafeau_link("https://f.io", "abc", "", "a.png", "image/png", true),
            "https://f.io/f.php?h=abc"
        );
    }

    #[test]
    fn maps_jirafeau_errors() {
        assert!(jirafeau_error("Error 2: No password nor allowed IP").contains("upload password"));
        assert!(jirafeau_error("Error 22").contains("expiry"));
        assert!(jirafeau_error("Error 13: The parameter password is required.").contains("download password"));
        assert_eq!(jirafeau_error("Error: cannot write file"), "Server error: Error: cannot write file");
    }

    /// Download what a link points at (`key` = download password), dropping the preview
    /// hint and forcing a plain download.
    async fn fetch(link: &str, key: Option<&str>) -> Vec<u8> {
        let url = link.split('#').next().unwrap().replace("&p=1", "") + "&d=1";
        let client = reqwest::Client::new();
        let req = match key {
            Some(k) => client.post(&url).form(&[("key", k)]),
            None => client.get(&url),
        };
        req.send().await.unwrap().bytes().await.unwrap().to_vec()
    }

    /// End-to-end against real Jirafeau servers: an open one, and one requiring the upload
    /// password `upsecret` with server-side encryption on. Run with
    /// `JIRAFEAU_TEST_OPEN=http://127.0.0.1:8099 JIRAFEAU_TEST_LOCKED=http://127.0.0.1:8100
    ///  cargo test --bin weechat-rs -- --ignored jirafeau_end_to_end`
    /// (PHP request limit of ~1 MB makes the 3 MB file go up in several chunks).
    #[tokio::test]
    #[ignore = "needs local Jirafeau servers (JIRAFEAU_TEST_OPEN / JIRAFEAU_TEST_LOCKED)"]
    async fn jirafeau_end_to_end() {
        let open = std::env::var("JIRAFEAU_TEST_OPEN").expect("JIRAFEAU_TEST_OPEN");
        let locked = std::env::var("JIRAFEAU_TEST_LOCKED").expect("JIRAFEAU_TEST_LOCKED");
        let dir = std::env::temp_dir().join(format!("weechatrs-jirafeau-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test pic.png");
        let data: Vec<u8> = (0..3 * 1024 * 1024 + 12_345u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let opts = |base: &str, up: &str, down: &str| JirafeauOptions {
            base_url: base.to_string(), upload_password: up.into(), download_password: down.into(),
        };
        let svc = FileShareService::Jirafeau;

        // Plain chunked upload: progress reaches the full size, link previews, bytes match.
        let progress = UploadProgress::new();
        let link = upload(path.clone(), &svc, "day", &opts(&open, "", ""), progress.clone()).await.unwrap();
        assert_eq!(progress.sent(), data.len() as u64);
        assert!(link.contains("&p=1#test%20pic.png"), "{link}");
        assert_eq!(fetch(&link, None).await, data, "downloaded bytes differ");

        // Expiry the server disabled → readable error.
        let err = upload(path.clone(), &svc, "year", &opts(&open, "", ""), UploadProgress::new()).await.unwrap_err();
        assert!(err.contains("expiry"), "{err}");

        // Upload password required / wrong.
        let err = upload(path.clone(), &svc, "day", &opts(&locked, "", ""), UploadProgress::new()).await.unwrap_err();
        assert!(err.contains("requires an upload password"), "{err}");
        let err = upload(path.clone(), &svc, "day", &opts(&locked, "nope", ""), UploadProgress::new()).await.unwrap_err();
        assert!(err.contains("rejected"), "{err}");

        // Encrypted server + download password: link carries the decryption key, no preview
        // hint, and only the right password gets the original bytes back.
        let link = upload(path.clone(), &svc, "day", &opts(&locked, "upsecret", "dlsecret"), UploadProgress::new()).await.unwrap();
        assert!(link.contains("&k=") && !link.contains("&p=1"), "{link}");
        assert_ne!(fetch(&link, None).await, data, "served without the download password");
        assert_eq!(fetch(&link, Some("dlsecret")).await, data, "decrypted bytes differ");

        // Cancelling stops the upload.
        let progress = UploadProgress::new();
        progress.cancel();
        let err = upload(path.clone(), &svc, "day", &opts(&open, "", ""), progress).await.unwrap_err();
        assert_eq!(err, CANCELLED);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fragment_encoding_and_sizes() {
        assert_eq!(encode_fragment("a b#c.png"), "a%20b%23c.png");
        assert_eq!(human_size(3 * 1024 * 1024 + 300 * 1024), "3.3 MB");
        assert_eq!(human_size(900), "900 B");
    }
}
