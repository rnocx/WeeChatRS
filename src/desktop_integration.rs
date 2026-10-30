//! Linux: make the desktop show WeeChatRS's icon even when the binary is run directly.
//!
//! Wayland has no way for a window to hand the compositor an icon; the compositor looks
//! up an installed `<app_id>.desktop` file instead (our app_id is `weechatrs`). Without
//! one, taskbars fall back to a generated placeholder. So on startup we install a
//! per-user desktop entry and icons into `$XDG_DATA_HOME` (`~/.local/share`), pointing
//! `Exec` at the running binary — the same files `packaging/linux/install.sh` installs.
//!
//! Files are only (re)written when missing or different. Skipped when a system-wide
//! entry exists, or when `WEECHATRS_NO_DESKTOP_INTEGRATION` is set.

use std::path::{Path, PathBuf};

const APP_ID: &str = "weechatrs";
const DESKTOP_TEMPLATE: &str = include_str!("../packaging/linux/weechatrs.desktop");
const ICONS: &[(&str, &[u8])] = &[
    ("256x256", include_bytes!("../assets/icon.iconset/icon_256x256.png")),
    ("512x512", include_bytes!("../assets/icon.iconset/icon_512x512.png")),
];

// Only called on Linux; the module is also compiled for tests elsewhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn ensure_installed() {
    if std::env::var_os("WEECHATRS_NO_DESKTOP_INTEGRATION").is_some() {
        return;
    }
    let system_entry = ["/usr/share/applications", "/usr/local/share/applications"]
        .iter()
        .any(|d| Path::new(d).join(format!("{APP_ID}.desktop")).exists());
    if system_entry {
        return;
    }
    if let Err(e) = install() {
        log::warn!("desktop integration: {e}");
    }
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

fn install() -> Result<(), String> {
    let data = data_home().ok_or("no XDG_DATA_HOME or HOME")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.canonicalize().unwrap_or(exe);

    let mut changed = false;
    for (size, png) in ICONS {
        let path = data.join(format!("icons/hicolor/{size}/apps/{APP_ID}.png"));
        changed |= write_if_changed(&path, png)?;
    }

    // Quote Exec per the Desktop Entry spec so paths with spaces still work.
    let exec = format!("Exec=\"{}\"", exe.display().to_string().replace('\\', "\\\\").replace('"', "\\\""));
    let desktop: String = DESKTOP_TEMPLATE
        .lines()
        .map(|l| if l.starts_with("Exec=") { exec.as_str() } else { l })
        .collect::<Vec<_>>()
        .join("\n") + "\n";
    let desktop_path = data.join(format!("applications/{APP_ID}.desktop"));
    changed |= write_if_changed(&desktop_path, desktop.as_bytes())?;

    if changed {
        log::info!("desktop integration: installed {}", desktop_path.display());
        // Best effort, and quiet: these tools' complaints are about other apps' files.
        let _ = quiet("update-desktop-database").arg(data.join("applications")).status();
        refresh_icon_theme(&data.join("icons/hicolor"));
    }
    Ok(())
}

fn quiet(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    command
}

/// Let the desktop see new icons in `theme`, as xdg-icon-resource does. A per-user
/// hicolor folder usually has no index.theme and no cache, and is read directly;
/// building a cache there would cover every app's icons in it, and gtk-update-icon-cache
/// refuses ("the generated cache was invalid") when any of them is off. So the cache is
/// only rebuilt where the theme has an index; elsewhere the folder's time is bumped,
/// which makes GTK treat an existing cache as stale.
fn refresh_icon_theme(theme: &Path) {
    if theme.join("index.theme").is_file() {
        let _ = quiet("gtk-update-icon-cache").arg("-q").arg(theme).status();
    } else if let Ok(dir) = std::fs::File::open(theme) {
        let _ = dir.set_modified(std::time::SystemTime::now());
    }
}

/// Write `contents` to `path` unless it already holds exactly that. Returns whether it wrote.
fn write_if_changed(path: &Path, contents: &[u8]) -> Result<bool, String> {
    if std::fs::read(path).is_ok_and(|existing| existing == contents) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn installs_entry_and_icons_once_into_data_home() {
        let dir = std::env::temp_dir().join(format!("weechatrs-desktop-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_DATA_HOME", &dir);

        super::install().expect("first install");
        let desktop = std::fs::read_to_string(dir.join("applications/weechatrs.desktop")).unwrap();
        let exe = std::env::current_exe().unwrap();
        let exe = exe.canonicalize().unwrap_or(exe);
        assert!(desktop.contains(&format!("Exec=\"{}\"", exe.display())), "{desktop}");
        assert!(desktop.contains("Icon=weechatrs"));
        for size in ["256x256", "512x512"] {
            assert!(dir.join(format!("icons/hicolor/{size}/apps/weechatrs.png")).is_file());
        }
        // No index.theme in a per-user folder: no icon cache is built there.
        assert!(!dir.join("icons/hicolor/icon-theme.cache").exists());

        // Unchanged files aren't rewritten.
        let before = std::fs::metadata(dir.join("applications/weechatrs.desktop")).unwrap().modified().unwrap();
        super::install().expect("second install");
        let after = std::fs::metadata(dir.join("applications/weechatrs.desktop")).unwrap().modified().unwrap();
        assert_eq!(before, after);

        std::env::remove_var("XDG_DATA_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn template_has_exec_and_matching_icon_name() {
        assert!(super::DESKTOP_TEMPLATE.lines().any(|l| l.starts_with("Exec=")));
        assert!(super::DESKTOP_TEMPLATE.lines().any(|l| l == format!("Icon={}", super::APP_ID)));
        assert!(super::DESKTOP_TEMPLATE.lines().any(|l| l == format!("StartupWMClass={}", super::APP_ID)));
    }
}
