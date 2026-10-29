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

/// Drops control and invisible format characters players leave in titles.
fn clean(s: &str) -> String {
    s.chars()
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
        .to_string()
}

/// "Artist - Title", or just the title when there is no artist.
fn track_of(artist: &str, title: &str) -> Option<String> {
    let (artist, title) = (clean(artist), clean(title));
    if title.is_empty() { return None; }
    Some(if artist.is_empty() { title } else { format!("{} - {}", artist, title) })
}

const NOTHING_PLAYING: &str = "Nothing is playing";

/// The playing track, or why there is none.
pub async fn get_now_playing() -> Result<NowPlaying, String> {
    #[cfg(target_os = "macos")]
    return macos::detect().await.ok_or_else(|| NOTHING_PLAYING.to_string());

    #[cfg(target_os = "linux")]
    return linux::detect().await;

    #[cfg(target_os = "windows")]
    return smtc::detect().await.ok_or_else(|| NOTHING_PLAYING.to_string());

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err("/np isn't supported on this platform".to_string());
}

#[cfg(target_os = "macos")]
mod macos {
    use std::time::Duration;
    use tokio::process::Command;
    use super::{NowPlaying, clean, search_url, track_of};

    // Reads what Control Centre's Now Playing widget shows, so any app that reports
    // playback (browsers included) is found without being asked, or launched. Since
    // macOS 15.4 MediaRemote answers only Apple-signed processes; osascript is one.
    // Prints nothing when nothing is playing, and fails when MediaRemote is missing.
    const NOW_PLAYING_JXA: &str = r#"
        ObjC.import("Foundation");
        $.NSBundle.bundleWithPath("/System/Library/PrivateFrameworks/MediaRemote.framework/").load;
        const R = $.NSClassFromString("MRNowPlayingRequest");
        if (R.isNil()) throw new Error("MRNowPlayingRequest unavailable");
        const str = (v) => { const u = ObjC.unwrap(v); return typeof u === "string" ? u : ""; };
        const item = R.localNowPlayingItem;
        if (!ObjC.unwrap(R.localIsPlaying) || item.isNil()) { ""; } else {
            const info = item.nowPlayingInfo;
            const path = R.localNowPlayingPlayerPath;
            const client = path.isNil() ? null : path.client;
            JSON.stringify({
                title: str(info.valueForKey("kMRMediaRemoteNowPlayingInfoTitle")),
                artist: str(info.valueForKey("kMRMediaRemoteNowPlayingInfoArtist")),
                bundle: client && !client.isNil() ? str(client.bundleIdentifier) : "",
                name: client && !client.isNil() ? str(client.displayName) : "",
            });
        }
    "#;

    // Browsers report playback under their own name; "@ Vivaldi" says nothing useful.
    const BROWSERS: &[&str] = &[
        "com.apple.Safari", "com.google.Chrome", "com.google.Chrome.canary",
        "org.chromium.Chromium", "com.vivaldi.Vivaldi", "com.brave.Browser",
        "com.microsoft.edgemac", "org.mozilla.firefox", "com.kagi.kagimacOS",
        "company.thebrowser.Browser", "com.operasoftware.Opera",
    ];

    #[derive(serde::Deserialize)]
    struct Info {
        title: String,
        artist: String,
        bundle: String,
        name: String,
    }

    /// Runs osascript, giving up after a few seconds so a hung app can't stall /np.
    async fn osascript(args: &[&str]) -> Result<String, ()> {
        let run = Command::new("osascript").args(args).kill_on_drop(true).output();
        let out = tokio::time::timeout(Duration::from_secs(3), run).await
            .map_err(|_| ())?.map_err(|_| ())?;
        if !out.status.success() { return Err(()); }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn source_of(bundle: &str, name: &str) -> String {
        let name = clean(name);
        let source = match bundle {
            "com.apple.Music" => "Apple Music",
            "com.spotify.client" => "Spotify",
            _ if name.contains("YouTube Music") => "YouTube Music",
            _ if name.contains("Spotify") => "Spotify",
            _ if BROWSERS.contains(&bundle) => "",
            _ => &name,
        };
        source.to_string()
    }

    /// The Spotify app's link to the playing track, when it is the one playing.
    async fn spotify_track_url() -> Option<String> {
        let uri = osascript(&["-e",
            "tell application id \"com.spotify.client\" to return spotify url of current track",
        ]).await.ok()?;
        uri.strip_prefix("spotify:track:").map(|id| format!("https://open.spotify.com/track/{}", id))
    }

    /// Ok(None) means nothing is playing; Err means MediaRemote couldn't be read.
    async fn media_remote() -> Result<Option<NowPlaying>, ()> {
        let json = osascript(&["-l", "JavaScript", "-e", NOW_PLAYING_JXA]).await?;
        if json.is_empty() { return Ok(None); }
        let info: Info = serde_json::from_str(&json).map_err(|_| ())?;
        let Some(track) = track_of(&info.artist, &info.title) else { return Ok(None) };
        let source = source_of(&info.bundle, &info.name);
        let url = match info.bundle.as_str() {
            "com.spotify.client" => spotify_track_url().await,
            _ => None,
        }.unwrap_or_else(|| search_url(&source, &track));
        Ok(Some(NowPlaying { track, source, url }))
    }

    /// Asks a player app directly. Checks it is running first: telling an app
    /// anything launches it, and one that isn't installed would prompt for it.
    async fn ask_app(app: &str, source: &str) -> Option<NowPlaying> {
        let running = osascript(&["-e", &format!("application \"{app}\" is running")]).await.ok()?;
        if running != "true" { return None; }
        let out = osascript(&["-e", &format!(
            "tell application \"{app}\" to if player state is playing \
             then return (artist of current track) & tab & (name of current track)"
        )]).await.ok()?;
        let (artist, title) = out.split_once('\t')?;
        let track = track_of(artist, title)?;
        let url = match source {
            "Spotify" => spotify_track_url().await,
            _ => None,
        }.unwrap_or_else(|| search_url(source, &track));
        Some(NowPlaying { track, source: source.to_string(), url })
    }

    pub async fn detect() -> Option<NowPlaying> {
        match media_remote().await {
            Ok(np) => np,
            Err(()) => {
                if let Some(np) = ask_app("Music", "Apple Music").await { return Some(np); }
                ask_app("Spotify", "Spotify").await
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::time::Duration;
    use tokio::process::Command;
    use super::{NOTHING_PLAYING, NowPlaying, clean, search_url, track_of};

    // Field separator for playerctl's --format; titles won't contain it.
    const SEP: char = '\u{1f}';

    /// The site a player's track URL points at, which says more than the player
    /// does: a browser could be playing YouTube, Twitch or a podcast.
    fn source_of_url(url: &str) -> Option<&'static str> {
        let host = url::Url::parse(url).ok()?.host_str()?.to_lowercase();
        let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
        Some(match host.as_str() {
            "music.youtube.com" => "YouTube Music",
            "youtube.com" | "m.youtube.com" | "youtu.be" => "YouTube",
            "open.spotify.com" => "Spotify",
            "soundcloud.com" => "SoundCloud",
            "twitch.tv" => "Twitch",
            _ => return None,
        })
    }

    fn source_of_player(player: &str) -> &'static str {
        if player.to_lowercase().contains("spotify") { "Spotify" } else { "" }
    }

    pub async fn detect() -> Result<NowPlaying, String> {
        let format = ["{{status}}", "{{playerName}}", "{{artist}}", "{{title}}", "{{xesam:url}}"]
            .join(&SEP.to_string());
        let run = Command::new("playerctl")
            .args(["--all-players", "metadata", "--format", &format])
            .kill_on_drop(true)
            .output();
        let out = match tokio::time::timeout(Duration::from_secs(3), run).await {
            Err(_) => return Err("the media player didn't answer".to_string()),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err("needs playerctl, which isn't installed".to_string());
            }
            Ok(Err(e)) => return Err(format!("couldn't run playerctl: {}", e)),
            Ok(Ok(out)) => out,
        };
        // Exits non-zero when there are no players at all.
        if !out.status.success() { return Err(NOTHING_PLAYING.to_string()); }

        // One line per player; take the first that is actually playing, not paused.
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            let mut parts = line.splitn(5, SEP);
            let status = parts.next().unwrap_or("").trim();
            let player = parts.next().unwrap_or("");
            let artist = parts.next().unwrap_or("");
            let title  = parts.next().unwrap_or("");
            let xesam_url = clean(parts.next().unwrap_or(""));

            if status != "Playing" { continue; }
            let Some(track) = track_of(artist, title) else { continue };

            let site = source_of_url(&xesam_url);
            let source = site.unwrap_or_else(|| source_of_player(player)).to_string();
            let url = if site.is_some() { xesam_url } else { search_url(&source, &track) };
            return Ok(NowPlaying { track, source, url });
        }
        Err(NOTHING_PLAYING.to_string())
    }
}

#[cfg(target_os = "windows")]
mod smtc {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession,
        GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    };
    use super::{NowPlaying, search_url, track_of};

    fn classify_source(app_id: &str) -> &'static str {
        let id = app_id.to_lowercase();
        if id.contains("applemusic") || id.contains("itunes") { "Apple Music" }
        else if id.contains("spotify") { "Spotify" }
        else if id.contains("youtube") || id.contains("chrome") || id.contains("msedge")
             || id.contains("brave") || id.contains("vivaldi") || id.contains("orion")
             || id.contains("firefox") { "YouTube Music" }
        else { "" }
    }

    async fn np_from_session(session: &GlobalSystemMediaTransportControlsSession) -> Option<NowPlaying> {
        let info = session.GetPlaybackInfo().ok()?;
        if info.PlaybackStatus().ok()? != PlaybackStatus::Playing { return None; }

        let props = session.TryGetMediaPropertiesAsync().ok()?.await.ok()?;
        let title = props.Title().ok()?.to_string();
        let artist = props.Artist().ok().map(|s| s.to_string()).unwrap_or_default();
        let track = track_of(&artist, &title)?;

        let app_id = session.SourceAppUserModelId().ok()
            .map(|s| s.to_string()).unwrap_or_default();
        let source = classify_source(&app_id).to_string();
        let url = search_url(&source, &track);
        Some(NowPlaying { track, source, url })
    }

    // Not Send (WinRT types), run via block_on inside spawn_blocking.
    async fn detect_winrt() -> Option<NowPlaying> {
        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
            .ok()?.await.ok()?;

        if let Ok(session) = manager.GetCurrentSession() {
            if let Some(np) = np_from_session(&session).await {
                return Some(np);
            }
        }

        let sessions = manager.GetSessions().ok()?;
        let count = sessions.Size().unwrap_or(0);
        let mut fallback: Option<NowPlaying> = None;

        for i in 0..count {
            if let Ok(session) = sessions.GetAt(i) {
                let app_id = session.SourceAppUserModelId().ok()
                    .map(|s| s.to_string()).unwrap_or_default();
                if let Some(np) = np_from_session(&session).await {
                    if !classify_source(&app_id).is_empty() {
                        return Some(np);
                    }
                    if fallback.is_none() {
                        fallback = Some(np);
                    }
                }
            }
        }

        fallback
    }

    pub async fn detect() -> Option<NowPlaying> {
        tokio::task::spawn_blocking(|| {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .ok()?
                .block_on(detect_winrt())
        })
        .await
        .ok()?
    }
}
