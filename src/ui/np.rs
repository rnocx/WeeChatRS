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

    // Chromium-family browsers: query all tabs for music.youtube.com.
    const CHROMIUM_BROWSERS: &[&str] = &[
        "Google Chrome",
        "Brave Browser",
        "Microsoft Edge",
        "Chromium",
        "Arc",
        "Vivaldi",
        "Opera",
    ];

    async fn run_script(script: &str) -> Option<String> {
        let out = Command::new("osascript").arg("-e").arg(script).output().await.ok()?;
        if !out.status.success() { return None; }
        // Strip control chars so embedded \r/\n in track metadata don't corrupt output.
        let s: String = String::from_utf8_lossy(&out.stdout)
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        if s.is_empty() || s.to_lowercase().contains("execution error") {
            return None;
        }
        Some(s)
    }

    /// Strip "- YouTube Music" (or "– YouTube Music") trailing suffix from a
    /// browser tab title, returning the cleaned track string, or `None` if the
    /// title doesn't look like a playing track.
    fn parse_ytm_title(title: &str) -> Option<String> {
        let cleaned = title
            .trim_end_matches(" - YouTube Music")
            .trim_end_matches(" \u{2013} YouTube Music") // en-dash variant
            .trim();
        // Bare "YouTube Music" means the page is open but nothing playing
        if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("YouTube Music") {
            return None;
        }
        Some(cleaned.to_string())
    }

    async fn check_chromium_browser(browser: &str) -> Option<String> {
        let script = format!(
            "tell application \"{browser}\"
                repeat with w in windows
                    repeat with t in tabs of w
                        if URL of t contains \"music.youtube.com/watch\" then
                            return title of t
                        end if
                    end repeat
                end repeat
            end tell"
        );
        let title = run_script(&script).await?;
        parse_ytm_title(&title)
    }

    async fn check_safari() -> Option<String> {
        let script =
            "tell application \"Safari\"
                repeat with w in windows
                    repeat with t in tabs of w
                        if URL of t contains \"music.youtube.com/watch\" then
                            return name of t
                        end if
                    end repeat
                end repeat
            end tell";
        let title = run_script(script).await?;
        parse_ytm_title(&title)
    }

    // YouTube Music installed as a PWA (via Chrome or Edge) runs as its own process.
    // Its window title follows the same "Track - YouTube Music" pattern.
    async fn check_ytm_pwa() -> Option<String> {
        let script =
            "tell application \"System Events\"
                set ytProcs to (every process whose displayed name contains \"YouTube Music\")
                if ytProcs is {} then return \"\"
                tell item 1 of ytProcs
                    if (count of windows) = 0 then return \"\"
                    return name of window 1
                end tell
            end tell";
        let title = run_script(script).await?;
        parse_ytm_title(&title)
    }

    pub async fn detect() -> Option<String> {
        // 1. Native music apps
        for script in NATIVE_PLAYERS {
            if let Some(s) = run_script(script).await {
                return Some(s);
            }
        }

        // 2. YouTube Music in browsers
        for browser in CHROMIUM_BROWSERS {
            if let Some(s) = check_chromium_browser(browser).await {
                return Some(s);
            }
        }
        if let Some(s) = check_safari().await {
            return Some(s);
        }

        // 3. YouTube Music PWA
        if let Some(s) = check_ytm_pwa().await {
            return Some(s);
        }

        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use tokio::process::Command;

    pub async fn detect() -> Option<String> {
        // playerctl handles both native MPRIS2 players (Spotify, VLC, mpd, ...)
        // and browsers with media integration (Chrome 73+, Firefox 82+),
        // which includes YouTube Music running in a browser tab.
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

        // Fallback: enumerate MPRIS2 services on the session bus and try each.
        if let Ok(list_out) = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.DBus",
                "--type=method_call",
                "--print-reply",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus.ListNames",
            ])
            .output()
            .await
        {
            let names = String::from_utf8_lossy(&list_out.stdout);
            for service in names.lines().filter_map(|l| {
                let t = l.trim().trim_matches('"');
                if t.starts_with("org.mpris.MediaPlayer2.") { Some(t.to_string()) } else { None }
            }) {
                let player_name = service
                    .strip_prefix("org.mpris.MediaPlayer2.")
                    .unwrap_or(&service);
                if let Ok(out) = Command::new("playerctl")
                    .args([
                        "--player", player_name,
                        "metadata", "--format", "{{ artist }} - {{ title }}",
                    ])
                    .output()
                    .await
                {
                    if out.status.success() {
                        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if !s.is_empty() && s != " - " {
                            return Some(s);
                        }
                    }
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
        // Strip all control characters (including embedded \r and \n that appear
        // when SMTC stores multi-value artist fields as newline-separated strings).
        // Joining without a separator preserves names split across lines (e.g.
        // "Hard Dr\niver" → "Hard Driver").
        let raw = String::from_utf8_lossy(&out.stdout);
        let s: String = raw
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        if s.is_empty() { None } else { Some(s) }
    }

    // Try SMTC via the "current" session first — simplest path, works when
    // Apple Music or another known player is the active media session.
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
$artist = $props.Artist; $title = $props.Title
if ([string]::IsNullOrWhiteSpace($artist)) { Write-Output $title } else { Write-Output "$artist - $title" }
"#;

    // Enumerate all SMTC sessions — catches players that aren't the Windows
    // "current" session. Browsers and known apps are preferred.
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
        $isKnown = $s.SourceAppUserModelId -match 'chrome|msedge|firefox|brave|vivaldi|opera|applemusic|itunes|spotify'
        if ($isKnown -and -not $bestIsKnown) { $best = $props; $bestIsKnown = $true }
        elseif ($null -eq $best) { $best = $props }
    } catch { }
}
if ($null -eq $best) { exit 1 }
$artist = $best.Artist; $title = $best.Title
if ([string]::IsNullOrWhiteSpace($artist)) { Write-Output $title } else { Write-Output "$artist - $title" }
"#;

    // Apple Music (Microsoft Store) window-title fallback.
    // Process name: AppleMusic. Title when playing: "Song – Artist – Apple Music"
    // or just "Song – Apple Music". We accept any non-empty title that isn't the
    // idle default ("Apple Music") and strip the trailing app name.
    const APPLE_MUSIC_SCRIPT: &str = r#"
$p = Get-Process -Name AppleMusic -ErrorAction SilentlyContinue |
     Where-Object { $_.MainWindowTitle -ne '' -and $_.MainWindowTitle -ne 'Apple Music' } |
     Select-Object -First 1
if ($null -eq $p) { exit 1 }
$t = $p.MainWindowTitle -replace '\s*[-–]+\s*Apple Music\s*$', ''
$t = $t.Trim()
if (-not $t) { exit 1 }
Write-Output $t
"#;

    // Fallback: find a browser window whose title contains "YouTube Music" and
    // strip the browser chrome, leaving "Track" (or "Track - Artist").
    // Only works when the YTM tab is the active tab in that browser window.
    const YTM_BROWSER_SCRIPT: &str = r#"
$browsers = 'chrome','msedge','brave','vivaldi','opera','firefox'
foreach ($b in $browsers) {
    $p = Get-Process -Name $b -ErrorAction SilentlyContinue |
         Where-Object { $_.MainWindowTitle -match 'YouTube Music' } |
         Select-Object -First 1
    if ($p) {
        $t = $p.MainWindowTitle -replace '\s*[-–]+\s*YouTube Music\b.*$', ''
        $t = $t.Trim()
        if ($t -and $t -ne 'YouTube Music') { Write-Output $t; exit 0 }
    }
}
exit 1
"#;

    // iTunes window title: "Artist – Title" while playing (classic desktop app).
    const ITUNES_TITLE_SCRIPT: &str = r#"
$p = Get-Process -Name iTunes -ErrorAction SilentlyContinue |
     Where-Object { $_.MainWindowTitle -ne '' -and $_.MainWindowTitle -ne 'iTunes' } |
     Select-Object -First 1
if ($null -eq $p) { exit 1 }
$t = $p.MainWindowTitle -replace '\s*[-–]+\s*iTunes\s*$', ''
$t = $t.Trim()
if (-not $t) { exit 1 }
Write-Output $t
"#;

    // Last resort: Spotify window title shows "Artist - Title" when playing.
    const SPOTIFY_TITLE_SCRIPT: &str = r#"
$p = Get-Process -Name Spotify -ErrorAction SilentlyContinue |
     Where-Object { $_.MainWindowTitle -ne '' -and $_.MainWindowTitle -ne 'Spotify' } |
     Select-Object -First 1
if ($null -eq $p) { exit 1 }
Write-Output $p.MainWindowTitle
"#;

    pub async fn detect() -> Option<String> {
        tokio::task::spawn_blocking(|| {
            run_ps(SMTC_CURRENT_SCRIPT)
                .or_else(|| run_ps(SMTC_ALL_SCRIPT))
                .or_else(|| run_ps(APPLE_MUSIC_SCRIPT))
                .or_else(|| run_ps(YTM_BROWSER_SCRIPT))
                .or_else(|| run_ps(ITUNES_TITLE_SCRIPT))
                .or_else(|| run_ps(SPOTIFY_TITLE_SCRIPT))
        })
        .await
        .ok()?
    }
}
