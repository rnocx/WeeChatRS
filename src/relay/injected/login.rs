//! User-initiated import of an existing Injected desktop session. We only read
//! Injected's own Local Storage files; browser profiles are never searched.
use base64::Engine as _;
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

pub struct ImportedSession {
    pub token: String,
    pub username: String,
}
#[derive(Default)]
pub struct ImportState {
    pub receiver: Option<mpsc::Receiver<Result<ImportedSession, String>>>,
    #[cfg(feature = "injected-web-login")]
    pub cancel: Option<tokio::sync::oneshot::Sender<()>>,
    pub message: Option<String>,
}

fn default_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    if let Some(base) = base {
        for name in ["injected", "Injected"] {
            roots.push(base.join(name));
        }
    }
    roots
}
fn storage_path(root: &Path) -> PathBuf {
    if root.file_name().and_then(|s| s.to_str()) == Some("leveldb") {
        root.into()
    } else {
        root.join("Local Storage/leveldb")
    }
}
fn tokens(bytes: &[u8]) -> Vec<String> {
    // Electron writes Latin-1 strings directly into the LevelDB journal. Only
    // candidates adjacent to Injected's token key are considered; stale sessions
    // are checked against /api/me before anything is offered to the user.
    let data = String::from_utf8_lossy(bytes);
    let regex = regex::Regex::new(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap();
    let mut out = Vec::new();
    for found in regex.find_iter(&data) {
        let from = data[..found.start()]
            .char_indices()
            .rev()
            .nth(8192)
            .map(|(i, _)| i)
            .unwrap_or(0);
        if !data[from..found.start()].contains("injected_token") {
            continue;
        }
        let candidate = found.as_str();
        if candidate.len() > 16384 {
            continue;
        }
        let payload = candidate.split('.').nth(1).and_then(|p| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(p)
                .ok()
        });
        let Some(payload) =
            payload.and_then(|p| serde_json::from_slice::<serde_json::Value>(&p).ok())
        else {
            continue;
        };
        if payload["exp"]
            .as_i64()
            .map(|exp| exp <= chrono::Utc::now().timestamp())
            .unwrap_or(false)
        {
            continue;
        }
        out.push(candidate.into());
    }
    out.reverse(); // most recent journal entries first
    out
}
fn find_tokens(roots: Vec<PathBuf>) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for root in roots {
        if let Ok(entries) = std::fs::read_dir(storage_path(&root)) {
            files.extend(
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| {
                        matches!(
                            p.extension().and_then(|s| s.to_str()),
                            Some("log" | "ldb" | "sst")
                        )
                    }),
            );
        }
    }
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    let mut result = Vec::new();
    let mut budget = 128 * 1024 * 1024u64;
    for path in files {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.len() > 32 * 1024 * 1024 || meta.len() > budget {
            continue;
        }
        budget -= meta.len();
        if let Ok(data) = std::fs::read(path) {
            for token in tokens(&data) {
                if !result.contains(&token) {
                    result.push(token);
                }
            }
        }
        if result.len() >= 8 {
            break;
        }
    }
    result.truncate(8);
    if result.is_empty() {
        Err("No readable session found. Sign in to the Injected desktop app, then try again. You can also choose its data folder or paste a token manually.".into())
    } else {
        Ok(result)
    }
}

pub fn start(
    folder: Option<PathBuf>,
    ctx: egui::Context,
) -> mpsc::Receiver<Result<ImportedSession, String>> {
    let (tx, rx) = mpsc::channel();
    tokio::spawn(async move {
        let result=async {
            let roots=folder.map(|p|vec![p]).unwrap_or_else(default_roots);
            let candidates=tokio::task::spawn_blocking(move||find_tokens(roots)).await.map_err(|_|"Could not read Injected's session data.".to_string())??;
            let client=reqwest::Client::builder().timeout(std::time::Duration::from_secs(8)).redirect(reqwest::redirect::Policy::none()).build().map_err(|_|"Could not initialize session verification.".to_string())?;
            for token in candidates {
                // Imports always validate against the official service; never
                // send an imported credential to an editable profile endpoint.
                let response=client.get("https://injected-irc.com/api/me").bearer_auth(&token).send().await.map_err(|_|"Could not reach Injected to verify the session. Try again when connected.".to_string())?;
                if matches!(response.status().as_u16(),401|403) {continue;}
                if !response.status().is_success() {return Err("Injected could not verify the session. Try again later.".into());}
                let bytes=response.bytes().await.map_err(|_|"Could not read session verification.".to_string())?;
                let user:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|"Unexpected session verification response.".to_string())?;
                let username=user["username"].as_str().filter(|s|!s.is_empty()).ok_or_else(||"Injected did not return an account name.".to_string())?.to_owned();
                return Ok(ImportedSession {token,username});
            }
            Err("The saved Injected session has expired. Sign in to the desktop app again, then retry.".into())
        }.await;
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    fn token(exp: i64) -> String {
        format!(
            "eyJhbGciOiJIUzI1NiJ9.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{{\"exp\":{exp}}}"))
        )
    }
    #[test]
    fn extracts_only_nearby_unexpired_injected_sessions() {
        let valid = token(chrono::Utc::now().timestamp() + 3600);
        let expired = token(1);
        assert!(tokens(valid.as_bytes()).is_empty());
        let bytes = format!("injected_token\x00{expired}\x00injected_token\x01{valid}");
        assert_eq!(tokens(bytes.as_bytes()), vec![valid]);
    }
    #[test]
    fn reads_only_injected_storage_files() {
        let dir = std::env::temp_dir().join(format!(
            "weechatrs-session-import-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let storage = dir.join("Local Storage/leveldb");
        std::fs::create_dir_all(&storage).unwrap();
        let valid = token(chrono::Utc::now().timestamp() + 3600);
        std::fs::write(
            storage.join("000001.log"),
            format!("injected_token\x01{valid}\x00"),
        )
        .unwrap();
        std::fs::write(storage.join("unrelated.txt"), "injected_token").unwrap();
        assert_eq!(find_tokens(vec![dir.clone()]).unwrap(), vec![valid]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn accepts_app_folder_or_leveldb_folder() {
        assert_eq!(
            storage_path(Path::new("app")),
            PathBuf::from("app/Local Storage/leveldb")
        );
        assert_eq!(
            storage_path(Path::new("app/Local Storage/leveldb")),
            PathBuf::from("app/Local Storage/leveldb")
        );
    }
}

/// Verify the session without exposing server response bodies or credentials.
#[cfg(feature = "injected-web-login")]
pub async fn verify_session(token: String) -> Result<ImportedSession, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize session verification.".to_string())?;
    let response = client
        .get("https://injected-irc.com/api/me")
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|_| "Could not reach Injected to verify sign-in. Please retry.".to_string())?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err("Injected rejected the session. Please sign in again.".into());
    }
    if !response.status().is_success() {
        return Err("Injected could not verify sign-in. Please retry later.".into());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Could not read sign-in verification.".to_string())?;
    let user: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Unexpected sign-in verification response.".to_string())?;
    let username = user["username"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Injected did not return an account name.".to_string())?
        .to_owned();
    Ok(ImportedSession { token, username })
}
