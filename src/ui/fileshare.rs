use std::path::PathBuf;

const UPLOAD_URL: &str = "https://files.interdo.me/script.php";
const DOWNLOAD_BASE: &str = "https://files.interdo.me/f.php";

/// Valid Jirafeau expiry keywords. Anything else falls back to "day".
const VALID_TIMES: &[&str] = &[
    "minute", "hour", "day", "week", "fortnight", "month", "quarter", "year", "none",
];

/// Upload `path` to the file-share service and return the public download URL on success.
pub async fn upload(path: PathBuf, duration: &str) -> Result<String, String> {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Cannot read file: {}", e))?;

    let mime = mime_for(&filename);

    let time = if VALID_TIMES.contains(&duration) { duration } else { "day" };

    let file_part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename)
        .mime_str(mime)
        .map_err(|e| format!("MIME error: {}", e))?;

    // Field order matches the bash script (time first, then file).
    // HTTP/1.1 only + Connection: close replicates curl's --http1.0 behaviour
    // which some PHP/nginx setups require for correct multipart parsing.
    let form = reqwest::multipart::Form::new()
        .text("time", time.to_string())
        .part("file", file_part);

    let client = reqwest::Client::builder()
        .http1_only()
        .connection_verbose(false)
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client
        .post(UPLOAD_URL)
        .header("Connection", "close")
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Upload failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // The server returns up to three lines:
    //   line 0: file code (hash)
    //   line 1: delete code
    //   line 2: key code (only if upload password was used)
    // Error responses start with "Error".
    let first_line = body.lines().next().unwrap_or("").trim();

    if first_line.starts_with("Error") || first_line.is_empty() {
        return Err(format!("Server error: {}", first_line));
    }

    Ok(format!("{}?h={}", DOWNLOAD_BASE, first_line))
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
