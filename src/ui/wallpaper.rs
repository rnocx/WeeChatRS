use crate::ui::theme::{AppTheme, ThemeColor};
use egui::Color32;

// ── Color math ────────────────────────────────────────────────────────────────

fn luminance(r: u8, g: u8, b: u8) -> f32 {
    0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
}

fn saturation_hsv(r: u8, g: u8, b: u8) -> f32 {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max == 0.0 { 0.0 } else { (max - min) / max }
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    if s < 1e-6 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let ch = |mut t: f32| -> f32 {
        if t < 0.0 { t += 1.0; }
        if t > 1.0 { t -= 1.0; }
        if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
        if t < 0.5 { return q; }
        if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
        p
    };
    (
        (ch(h + 1.0 / 3.0) * 255.0).round() as u8,
        (ch(h) * 255.0).round() as u8,
        (ch(h - 1.0 / 3.0) * 255.0).round() as u8,
    )
}

fn tc(r: u8, g: u8, b: u8) -> ThemeColor {
    ThemeColor::from(Color32::from_rgb(r, g, b))
}

fn tc_hsl(h: f32, s: f32, l: f32) -> ThemeColor {
    let (r, g, b) = hsl_to_rgb(h, s, l);
    tc(r, g, b)
}

// ── K-means color palette extraction ─────────────────────────────────────────

/// Returns (r, g, b, cluster_size) sorted by descending cluster size.
fn kmeans_palette(pixels: &[(u8, u8, u8)], k: usize) -> Vec<(u8, u8, u8, usize)> {
    let k = k.min(pixels.len());
    if k == 0 {
        return Vec::new();
    }

    // Seed centroids with evenly-spaced samples across the pixel list.
    let mut centroids: Vec<(f32, f32, f32)> = (0..k)
        .map(|i| {
            let p = pixels[i * pixels.len() / k];
            (p.0 as f32, p.1 as f32, p.2 as f32)
        })
        .collect();

    let mut assignments = vec![0usize; pixels.len()];

    for _ in 0..20 {
        let mut changed = false;
        for (idx, &(r, g, b)) in pixels.iter().enumerate() {
            let best = (0..k)
                .min_by_key(|&c| {
                    let dr = centroids[c].0 - r as f32;
                    let dg = centroids[c].1 - g as f32;
                    let db = centroids[c].2 - b as f32;
                    ((dr * dr + dg * dg + db * db) * 100.0) as u64
                })
                .unwrap_or(0);
            if assignments[idx] != best {
                assignments[idx] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }

        let mut sums = vec![(0f64, 0f64, 0f64, 0usize); k];
        for (idx, &(r, g, b)) in pixels.iter().enumerate() {
            let c = assignments[idx];
            sums[c].0 += r as f64;
            sums[c].1 += g as f64;
            sums[c].2 += b as f64;
            sums[c].3 += 1;
        }
        for (i, &(sr, sg, sb, n)) in sums.iter().enumerate() {
            if n > 0 {
                centroids[i] = ((sr / n as f64) as f32, (sg / n as f64) as f32, (sb / n as f64) as f32);
            }
        }
    }

    let mut counts = vec![0usize; k];
    for &a in &assignments {
        counts[a] += 1;
    }

    let mut result: Vec<(u8, u8, u8, usize)> = centroids
        .iter()
        .zip(counts.iter())
        .map(|(&(r, g, b), &n)| (r as u8, g as u8, b as u8, n))
        .collect();
    result.sort_by(|a, b| b.3.cmp(&a.3));
    result
}

// ── Image loading (platform-aware) ────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
fn load_image_rgb(path: &str) -> Option<Vec<(u8, u8, u8)>> {
    let img = image::open(path).ok()?;
    let img = img.thumbnail(128, 128).to_rgb8();
    Some(img.pixels().map(|p| (p.0[0], p.0[1], p.0[2])).collect())
}

#[cfg(target_os = "macos")]
fn load_image_rgb(path: &str) -> Option<Vec<(u8, u8, u8)>> {
    // Try formats the image crate supports (JPEG, PNG, WebP, …) directly.
    if let Ok(img) = image::open(path) {
        let img = img.thumbnail(128, 128).to_rgb8();
        return Some(img.pixels().map(|p| (p.0[0], p.0[1], p.0[2])).collect());
    }
    // HEIC/HEIF fallback: convert to a temp JPEG via macOS's built-in sips tool.
    let tmp = std::env::temp_dir().join("weechat_wallpaper_sample.jpg");
    let ok = std::process::Command::new("sips")
        .args(["-s", "format", "jpeg", "-z", "128", "128", path, "--out"])
        .arg(&tmp)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        return None;
    }
    let img = image::open(&tmp).ok()?;
    let img = img.thumbnail(128, 128).to_rgb8();
    Some(img.pixels().map(|p| (p.0[0], p.0[1], p.0[2])).collect())
}

// ── Wallpaper path detection ──────────────────────────────────────────────────

/// Returns the path of the current desktop wallpaper on the primary display.
pub fn get_wallpaper_path() -> Option<String> {
    platform_wallpaper_path()
}

#[cfg(target_os = "macos")]
fn platform_wallpaper_path() -> Option<String> {
    let out = std::process::Command::new("osascript")
        .args(["-e", "tell app \"Finder\" to POSIX path of (get desktop picture as alias)"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(target_os = "windows")]
fn platform_wallpaper_path() -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg")
        .args(["query", "HKCU\\Control Panel\\Desktop", "/v", "Wallpaper"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    // Each line looks like: "    Wallpaper    REG_SZ    C:\path\to\file.jpg"
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Wallpaper") {
            if let Some(idx) = t.to_uppercase().find("REG_SZ") {
                let path = t[idx + 6..].trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn platform_wallpaper_path() -> Option<String> {
    let de = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();

    if de.contains("kde") || de.contains("plasma") {
        if let Some(p) = kde_wallpaper() { return Some(p); }
    }
    if de.contains("xfce") {
        if let Some(p) = xfce_wallpaper() { return Some(p); }
    }
    // GNOME, Cinnamon, Unity, Budgie, Pantheon, or unknown — try gsettings.
    for key in &["picture-uri-dark", "picture-uri"] {
        if let Some(p) = gsettings_wallpaper(key) { return Some(p); }
    }
    None
}

#[cfg(target_os = "linux")]
fn gsettings_wallpaper(key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.background", key])
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    let raw = String::from_utf8(out.stdout).ok()?;
    let raw = raw.trim().trim_matches('\'');
    let path = if let Some(rest) = raw.strip_prefix("file://") {
        percent_decode(rest)
    } else {
        raw.to_string()
    };
    if path.is_empty() || path == "(not set)" { None } else { Some(path) }
}

#[cfg(target_os = "linux")]
fn kde_wallpaper() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let cfg = format!("{}/.config/plasma-org.kde.plasma.desktop-appletsrc", home);
    let content = std::fs::read_to_string(cfg).ok()?;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("Image=") {
            let path = val.strip_prefix("file://").unwrap_or(val).to_string();
            if !path.is_empty() { return Some(path); }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn xfce_wallpaper() -> Option<String> {
    let list_out = std::process::Command::new("xfconf-query")
        .args(["-c", "xfce4-desktop", "-l"])
        .output()
        .ok()?;
    let props = String::from_utf8(list_out.stdout).ok()?;
    for prop in props.lines() {
        let p = prop.trim();
        if p.contains("last-image") || p.contains("image-path") {
            if let Ok(val_out) = std::process::Command::new("xfconf-query")
                .args(["-c", "xfce4-desktop", "-p", p])
                .output()
            {
                let val = String::from_utf8(val_out.stdout).ok()?.trim().to_string();
                if !val.is_empty() { return Some(val); }
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

// ── Theme generation ──────────────────────────────────────────────────────────

fn palette_to_theme(palette: &[(u8, u8, u8, usize)]) -> AppTheme {
    // Determine dark vs light by the most dominant colour's luminance.
    let (dr, dg, db, _) = palette[0];
    let is_light = luminance(dr, dg, db) > 128.0;

    // Sort palette by luminance to find extremes.
    let mut by_luma: Vec<(u8, u8, u8)> = palette.iter().map(|&(r, g, b, _)| (r, g, b)).collect();
    by_luma.sort_by(|&(r1, g1, b1), &(r2, g2, b2)| {
        luminance(r1, g1, b1).partial_cmp(&luminance(r2, g2, b2)).unwrap()
    });

    // Background: extremal colour pushed to a readable darkness/lightness.
    let bg_raw = if is_light { by_luma.last() } else { by_luma.first() }
        .copied()
        .unwrap_or((18, 18, 18));
    let (bh, bs, _) = rgb_to_hsl(bg_raw.0, bg_raw.1, bg_raw.2);
    let bg_l = if is_light { 0.91_f32 } else { 0.10_f32 };
    let (bgr, bgg, bgb) = hsl_to_rgb(bh, bs * 0.35, bg_l); // Desaturate for a subtle tint.

    // Foreground: plain high-contrast neutral.
    let (fgr, fgg, fgb): (u8, u8, u8) = if is_light { (25, 25, 25) } else { (215, 215, 215) };

    // Most saturated colour drives accent hue, ANSI saturation, and UI chrome.
    let &(ar, ag, ab, _) = palette
        .iter()
        .max_by(|&&(r1, g1, b1, _), &&(r2, g2, b2, _)| {
            saturation_hsv(r1, g1, b1).partial_cmp(&saturation_hsv(r2, g2, b2)).unwrap()
        })
        .unwrap_or(&palette[0]);
    let (acc_h, _, _) = rgb_to_hsl(ar, ag, ab);
    let sat = saturation_hsv(ar, ag, ab).clamp(0.45, 0.88);

    // ANSI hues: red, green, yellow stay fixed for readability.
    // Slot 4 (the "blue" position) is replaced with the wallpaper's actual dominant
    // vivid hue — this is what app.rs uses as accent_color for all UI chrome.
    let ansi_hues: [f32; 6] = [
        0.000, // red
        0.333, // green
        0.167, // yellow
        acc_h, // accent slot: wallpaper's most-saturated hue → UI chrome colour
        0.833, // magenta
        0.500, // cyan
    ];
    let (l_normal, l_bright) = if is_light {
        (0.38_f32, 0.27_f32) // Darker for readability on light backgrounds.
    } else {
        (0.55_f32, 0.72_f32) // Slightly lighter than pure 0.50 to avoid overly dark accents.
    };

    let mut ansi: Vec<ThemeColor> = Vec::with_capacity(16);

    // 0: black — very dark tint of background hue.
    ansi.push(tc_hsl(bh, bs * 0.4, if is_light { 0.82 } else { 0.10 }));
    // 1–6: normal chromatic.
    for &h in &ansi_hues {
        ansi.push(tc_hsl(h, sat, l_normal));
    }
    // 7: dim white.
    ansi.push(tc_hsl(0.0, 0.0, if is_light { 0.38 } else { 0.72 }));

    // 8: bright black.
    ansi.push(tc_hsl(bh, bs * 0.3, if is_light { 0.60 } else { 0.35 }));
    // 9–14: bright chromatic.
    for &h in &ansi_hues {
        ansi.push(tc_hsl(h, sat.min(0.92), l_bright));
    }
    // 15: bright white.
    ansi.push(tc_hsl(0.0, 0.0, if is_light { 0.05 } else { 0.95 }));

    AppTheme {
        name: "Adaptive".to_string(),
        ansi,
        background: Some(tc(bgr, bgg, bgb)),
        foreground: Some(tc(fgr, fgg, fgb)),
    }
}

/// Derive an AppTheme from the image at `path`. Returns None if the image
/// cannot be opened or is too small to extract meaningful colours.
pub fn theme_from_wallpaper(path: &str) -> Option<AppTheme> {
    let pixels = load_image_rgb(path)?;
    if pixels.is_empty() {
        return None;
    }
    let palette = kmeans_palette(&pixels, 8);
    if palette.is_empty() {
        return None;
    }
    Some(palette_to_theme(&palette))
}

// ── Background watcher thread ─────────────────────────────────────────────────

/// macOS: stat the desktop-preferences plist to detect changes without spawning
/// osascript on every tick. Returns true when the mtime changed (or stat failed).
#[cfg(target_os = "macos")]
fn pref_file_changed(last_mtime: &mut Option<std::time::SystemTime>) -> bool {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return true,
    };
    // Check both the classic and the newer (Sonoma+) preferences files.
    let candidates = [
        format!("{}/Library/Preferences/com.apple.desktop.plist", home),
        format!("{}/Library/Application Support/com.apple.wallpaper/configuration.plist", home),
    ];
    let newest = candidates
        .iter()
        .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .max();
    match newest {
        Some(mtime) => {
            if last_mtime.map_or(true, |t| t != mtime) {
                *last_mtime = Some(mtime);
                true
            } else {
                false
            }
        }
        // Can't stat either file (e.g. future macOS change) — always check.
        None => true,
    }
}

/// Windows / Linux: path detection is fast enough to run on every tick.
#[cfg(not(target_os = "macos"))]
fn pref_file_changed(_last_mtime: &mut Option<std::time::SystemTime>) -> bool {
    true
}

/// Spawn a background thread that polls for wallpaper changes and sends a
/// freshly-derived AppTheme whenever the wallpaper path changes.
/// Polls every 2 seconds so changes are reflected quickly.
/// The returned Receiver must be drained each frame via `try_recv`.
pub fn start_wallpaper_thread(ctx: egui::Context) -> std::sync::mpsc::Receiver<AppTheme> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("wallpaper-watcher".into())
        .spawn(move || {
            let mut last_path = String::new();
            let mut last_mtime: Option<std::time::SystemTime> = None;
            loop {
                if pref_file_changed(&mut last_mtime) {
                    if let Some(path) = get_wallpaper_path() {
                        if path != last_path {
                            last_path = path.clone();
                            if let Some(theme) = theme_from_wallpaper(&path) {
                                if tx.send(theme).is_err() {
                                    break;
                                }
                                ctx.request_repaint();
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
    rx
}
