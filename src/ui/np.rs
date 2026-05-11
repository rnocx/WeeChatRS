/// Detect the currently playing track from the system media player.
///
/// Returns a formatted string (usually "Artist - Title"), or `None` when
/// nothing is playing or no supported player is found.
pub async fn get_now_playing() -> Option<String> {
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

    // Native app AppleScripts — each returns "Artist - Title" or empty.
    const NATIVE_PLAYERS: &[&str] = &[
        "tell application \"Music\" to if player state is playing \
         then return (artist of current track) & \" - \" & (name of current track)",
        "tell application \"Spotify\" to if player state is playing \
         then return (artist of current track) & \" - \" & (name of current track)",
        "tell application \"Vox\" to return (artist) & \" - \" & (track)",
    ];

    async fn run_script(script: &str) -> Option<String> {
        let out = Command::new("osascript").arg("-e").arg(script).output().await.ok()?;
        if !out.status.success() { return None; }
        // Strip control chars and invisible Unicode format chars (BOM, zero-width
        // spaces, etc.) that Apple Music can embed in artist/title metadata.
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

    // YouTube Music PWA (installed via Chrome or Edge) runs as its own process.
    async fn check_ytm_pwa() -> Option<String> {
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
        strip_app_suffix(&title, "YouTube Music").map(str::to_string)
    }

    // Spotify PWA (installed via Chrome or Edge) runs as its own process.
    // Native Spotify is already handled by NATIVE_PLAYERS above.
    async fn check_spotify_pwa() -> Option<String> {
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
        strip_app_suffix(&title, "Spotify").map(str::to_string)
    }

    pub async fn detect() -> Option<String> {
        for script in NATIVE_PLAYERS {
            if let Some(s) = run_script(script).await {
                return Some(s);
            }
        }
        if let Some(s) = check_ytm_pwa().await { return Some(s); }
        if let Some(s) = check_spotify_pwa().await { return Some(s); }
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use tokio::process::Command;

    pub async fn detect() -> Option<String> {
        // playerctl handles MPRIS2 players: Spotify, VLC, mpd, YouTube Music PWA, etc.
        if let Ok(out) = Command::new("playerctl")
            .args(["metadata", "--format", "{{ artist }} - {{ title }}"])
            .output()
            .await
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() && !s.contains("No players found") && s != " - " {
                    return Some(s);
                }
            }
        }
        None
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    fn run_ps(script: &str) -> Option<String> {
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !out.status.success() { return None; }
        // Strip control chars (Cc) and invisible Unicode format chars (Cf: BOM,
        // zero-width spaces, joiners) that Apple Music embeds in SMTC metadata.
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

    // Try the active SMTC session first — fastest path.
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
if ([string]::IsNullOrWhiteSpace($artist)) { Write-Output $title } else { Write-Output "$artist - $title" }
"#;

    // Enumerate all SMTC sessions — catches players that aren't the active session.
    // PlaybackStatus 4 = Playing.
    const SMTC_ALL_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager,
         Windows.Media.Control, ContentType=WindowsRuntime]
$mgr = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()
$mgr.AsTask().Wait()
$sessions = $mgr.Result.GetSessions()
$best = $null
$bestIsKnown = $false
foreach ($s in $sessions) {
    try {
        if ($s.GetPlaybackInfo().PlaybackStatus -ne 4) { continue }
        $pi = $s.TryGetMediaPropertiesAsync(); $pi.AsTask().Wait()
        $props = $pi.Result
        if ([string]::IsNullOrWhiteSpace($props.Title)) { continue }
        $isKnown = $s.SourceAppUserModelId -match 'chrome|msedge|applemusic|itunes|spotify'
        if ($isKnown -and -not $bestIsKnown) { $best = $props; $bestIsKnown = $true }
        elseif ($null -eq $best) { $best = $props }
    } catch { }
}
if ($null -eq $best) { exit 1 }
$artist = ([string]$best.Artist -replace '[\p{Cc}\p{Cf}]', '').Trim()
$title  = ([string]$best.Title  -replace '[\p{Cc}\p{Cf}]', '').Trim()
if ([string]::IsNullOrWhiteSpace($artist)) { Write-Output $title } else { Write-Output "$artist - $title" }
"#;

    pub async fn detect() -> Option<String> {
        tokio::task::spawn_blocking(|| {
            run_ps(SMTC_CURRENT_SCRIPT)
                .or_else(|| run_ps(SMTC_ALL_SCRIPT))
        })
        .await
        .ok()?
    }
}
