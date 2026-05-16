pub struct NowPlaying {
    pub track: String,
    pub source: String,
    pub url: String,
}

fn search_url(source: &str, track: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(track.as_bytes()).collect();
    match source {
        "Apple Music" => format!("https://music.apple.com/search?term={}", encoded),
        "Spotify" => format!("https://open.spotify.com/search/{}", encoded),
        "YouTube Music" => format!("https://music.youtube.com/search?q={}", encoded),
        _ => String::new(),
    }
}

pub async fn get_now_playing() -> Option<NowPlaying> {
    #[cfg(target_os = "macos")]
    return macos::detect().await;

    #[cfg(target_os = "linux")]
    return linux::detect().await;

    #[cfg(target_os = "windows")]
    return windows::detect().await;

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return None;
}

#[cfg(target_os = "macos")]
mod macos {
    use tokio::process::Command;
    use super::{NowPlaying, search_url};

    async fn run_script(script: &str) -> Option<String> {
        let out = Command::new("osascript").arg("-e").arg(script).output().await.ok()?;
        if !out.status.success() { return None; }
        let s: String = String::from_utf8_lossy(&out.stdout)
            .chars()
            .filter(|&c| {
                if c.is_control() { return false; }
                !matches!(c as u32,
                    0x00AD | 0x034F |
                    0x200B..=0x200F |
                    0x2028 | 0x2029 |
                    0xFEFF |
                    0xFFF9..=0xFFFB
                )
            })
            .collect::<String>()
            .trim()
            .to_string();
        if s.is_empty() || s.to_lowercase().contains("execution error") {
            return None;
        }
        Some(s)
    }

    fn strip_app_suffix<'a>(title: &'a str, app: &str) -> Option<&'a str> {
        let cleaned = title
            .trim_end_matches(&format!(" - {app}"))
            .trim_end_matches(&format!(" \u{2013} {app}"))
            .trim();
        if cleaned.is_empty() || cleaned.eq_ignore_ascii_case(app) {
            None
        } else {
            Some(cleaned)
        }
    }

    async fn check_music_app() -> Option<NowPlaying> {
        let track = run_script(
            "tell application \"Music\" to if player state is playing \
             then return (artist of current track) & \" - \" & (name of current track)"
        ).await?;
        let url = search_url("Apple Music", &track);
        Some(NowPlaying { track, source: "Apple Music".to_string(), url })
    }

    async fn check_spotify_native() -> Option<NowPlaying> {
        let track = run_script(
            "tell application \"Spotify\" to if player state is playing \
             then return (artist of current track) & \" - \" & (name of current track)"
        ).await?;
        // Try to get the real track URL (spotify:track:XXX → https://open.spotify.com/track/XXX)
        let url = if let Some(uri) = run_script(
            "tell application \"Spotify\" to if player state is playing \
             then return spotify url of current track"
        ).await {
            if let Some(id) = uri.strip_prefix("spotify:track:") {
                format!("https://open.spotify.com/track/{}", id)
            } else {
                search_url("Spotify", &track)
            }
        } else {
            search_url("Spotify", &track)
        };
        Some(NowPlaying { track, source: "Spotify".to_string(), url })
    }

    async fn check_vox() -> Option<NowPlaying> {
        let track = run_script(
            "tell application \"Vox\" to return (artist) & \" - \" & (track)"
        ).await?;
        Some(NowPlaying { track, source: "Vox".to_string(), url: String::new() })
    }

    async fn check_ytm_pwa() -> Option<NowPlaying> {
        let script =
            "tell application \"System Events\"
                set ps to (every process whose displayed name contains \"YouTube Music\")
                if ps is {} then return \"\"
                tell item 1 of ps
                    if (count of windows) = 0 then return \"\"
                    return name of window 1
                end tell
            end tell";
        let title = run_script(script).await?;
        let track = strip_app_suffix(&title, "YouTube Music")?.to_string();
        let url = search_url("YouTube Music", &track);
        Some(NowPlaying { track, source: "YouTube Music".to_string(), url })
    }

    async fn check_spotify_pwa() -> Option<NowPlaying> {
        let script =
            "tell application \"System Events\"
                set ps to (every process whose displayed name contains \"Spotify\")
                if ps is {} then return \"\"
                tell item 1 of ps
                    if (count of windows) = 0 then return \"\"
                    return name of window 1
                end tell
            end tell";
        let title = run_script(script).await?;
        let track = strip_app_suffix(&title, "Spotify")?.to_string();
        let url = search_url("Spotify", &track);
        Some(NowPlaying { track, source: "Spotify".to_string(), url })
    }

    pub async fn detect() -> Option<NowPlaying> {
        if let Some(np) = check_music_app().await { return Some(np); }
        if let Some(np) = check_spotify_native().await { return Some(np); }
        if let Some(np) = check_vox().await { return Some(np); }
        if let Some(np) = check_ytm_pwa().await { return Some(np); }
        if let Some(np) = check_spotify_pwa().await { return Some(np); }
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use tokio::process::Command;
    use super::{NowPlaying, search_url};

    fn map_player_name(player: &str) -> &'static str {
        let p = player.to_lowercase();
        if p.contains("spotify") { "Spotify" }
        else if p.contains("youtube") || p.contains("chrome") || p.contains("chromium")
            || p.contains("msedge") || p.contains("brave") || p.contains("orion")
            || p.contains("vivaldi") || p.contains("firefox")
        { "YouTube Music" }
        else { "" }
    }

    async fn playerctl(args: &[&str]) -> Option<String> {
        let out = Command::new("playerctl")
            .args(args)
            .output()
            .await
            .ok()?;
        if !out.status.success() { return None; }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() || s.contains("No players found") { None } else { Some(s) }
    }

    pub async fn detect() -> Option<NowPlaying> {
        // Use separate fields so we can handle a missing artist without producing " - title".
        let raw = playerctl(&[
            "metadata",
            "--format",
            "{{ playerName }}|||{{ artist }}|||{{ title }}|||{{ xesam:url }}",
        ]).await?;

        let mut parts = raw.splitn(4, "|||");
        let player_id  = parts.next().unwrap_or("").trim();
        let artist     = parts.next().unwrap_or("").trim();
        let title      = parts.next().unwrap_or("").trim();
        let xesam_url  = parts.next().unwrap_or("").trim();

        if title.is_empty() { return None; }

        let track = if artist.is_empty() {
            title.to_string()
        } else {
            format!("{} - {}", artist, title)
        };

        let source = map_player_name(player_id);
        let url = if !xesam_url.is_empty()
            && (xesam_url.contains("music.youtube.com") || xesam_url.contains("open.spotify.com"))
        {
            xesam_url.to_string()
        } else {
            search_url(source, &track)
        };

        Some(NowPlaying { track, source: source.to_string(), url })
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::os::windows::process::CommandExt;
    use super::{NowPlaying, search_url};

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    fn run_ps(script: &str) -> Option<String> {
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !out.status.success() { return None; }
        let raw = String::from_utf8_lossy(&out.stdout);
        let s: String = raw
            .chars()
            .filter(|&c| {
                if c.is_control() { return false; }
                !matches!(c as u32,
                    0x00AD |
                    0x034F |
                    0x200B..=0x200F |
                    0x2028 | 0x2029 |
                    0xFEFF |
                    0xFFF9..=0xFFFB
                )
            })
            .collect::<String>()
            .trim()
            .to_string();
        if s.is_empty() { None } else { Some(s) }
    }

    fn classify_source(app_id: &str) -> &'static str {
        let id = app_id.to_lowercase();
        if id.contains("applemusic") || id.contains("itunes") { "Apple Music" }
        else if id.contains("spotify") { "Spotify" }
        else if id.contains("youtube") || id.contains("chrome") || id.contains("msedge")
             || id.contains("brave") || id.contains("vivaldi") || id.contains("orion")
             || id.contains("firefox") { "YouTube Music" }
        else { "" }
    }

    // Output format from PS scripts: "src|||artist|||title"
    fn parse_output(raw: &str) -> (String, String) {
        let mut parts = raw.splitn(3, "|||");
        let src_id = parts.next().unwrap_or("").trim();
        let artist  = parts.next().unwrap_or("").trim();
        let title   = parts.next().unwrap_or("").trim();
        let source  = classify_source(src_id).to_string();
        let track = if artist.is_empty() {
            title.to_string()
        } else {
            format!("{} - {}", artist, title)
        };
        (source, track)
    }

    const SMTC_CURRENT_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager,
         Windows.Media.Control, ContentType=WindowsRuntime]
$mgr = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()
$mgr.AsTask().Wait()
$session = $mgr.Result.GetCurrentSession()
if ($null -eq $session) { exit 1 }
if ($session.GetPlaybackInfo().PlaybackStatus -ne 4) { exit 1 }
$pi = $session.TryGetMediaPropertiesAsync(); $pi.AsTask().Wait()
$props = $pi.Result
if ([string]::IsNullOrWhiteSpace($props.Title)) { exit 1 }
$artist = ([string]$props.Artist -replace '[\p{Cc}\p{Cf}]', '').Trim()
$title  = ([string]$props.Title  -replace '[\p{Cc}\p{Cf}]', '').Trim()
$src = $session.SourceAppUserModelId
Write-Output "$src|||$artist|||$title"
"#;

    const SMTC_ALL_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager,
         Windows.Media.Control, ContentType=WindowsRuntime]
$mgr = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()
$mgr.AsTask().Wait()
$sessions = $mgr.Result.GetSessions()
$best = $null
$bestSrc = ""
$bestIsKnown = $false
foreach ($s in $sessions) {
    try {
        if ($s.GetPlaybackInfo().PlaybackStatus -ne 4) { continue }
        $pi = $s.TryGetMediaPropertiesAsync(); $pi.AsTask().Wait()
        $props = $pi.Result
        if ([string]::IsNullOrWhiteSpace($props.Title)) { continue }
        $isKnown = $s.SourceAppUserModelId -match 'youtube|chrome|msedge|brave|vivaldi|orion|firefox|applemusic|itunes|spotify'
        if ($isKnown -and -not $bestIsKnown) { $best = $props; $bestSrc = $s.SourceAppUserModelId; $bestIsKnown = $true }
        elseif ($null -eq $best) { $best = $props; $bestSrc = $s.SourceAppUserModelId }
    } catch { }
}
if ($null -eq $best) { exit 1 }
$artist = ([string]$best.Artist -replace '[\p{Cc}\p{Cf}]', '').Trim()
$title  = ([string]$best.Title  -replace '[\p{Cc}\p{Cf}]', '').Trim()
Write-Output "$bestSrc|||$artist|||$title"
"#;

    pub async fn detect() -> Option<NowPlaying> {
        let raw = tokio::task::spawn_blocking(|| {
            run_ps(SMTC_CURRENT_SCRIPT).or_else(|| run_ps(SMTC_ALL_SCRIPT))
        })
        .await
        .ok()??;

        let (source, track) = parse_output(&raw);
        if track.is_empty() { return None; }
        let url = search_url(&source, &track);
        Some(NowPlaying { track, source, url })
    }
}
