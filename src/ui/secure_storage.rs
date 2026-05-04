use keyring::Entry;

const SERVICE: &str = "weechat-rs";

// On Linux, keyring uses zbus::blocking which calls block_on internally.
// The egui update loop runs inside a tokio runtime (CachedParkThread::block_on
// from #[tokio::main]), so any attempt to call block_on from it will panic.
// Spawning a dedicated OS thread gives zbus a clean stack with no runtime context.
// We block on the result via a channel — these calls are infrequent (button clicks only).
#[cfg(target_os = "linux")]
fn run_keyring<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv().map_err(|_| "keyring helper thread panicked or was dropped".to_string())
}

#[allow(dead_code)]
fn user_key(host: &str, port: &str) -> String {
    format!("{}:{}", host, port)
}

#[allow(dead_code)]
pub fn save(host: &str, port: &str, password: &str) -> Result<(), String> {
    let key = user_key(host, port);
    let password = password.to_string();
    #[cfg(target_os = "linux")]
    return run_keyring(move || {
        Entry::new(SERVICE, &key)
            .and_then(|e| e.set_password(&password))
            .map_err(|e| e.to_string())
    }).and_then(|r| r);
    #[cfg(not(target_os = "linux"))]
    Entry::new(SERVICE, &key)
        .and_then(|e| e.set_password(&password))
        .map_err(|e| e.to_string())
}

#[allow(dead_code)]
pub fn load(host: &str, port: &str) -> Option<String> {
    let key = user_key(host, port);
    #[cfg(target_os = "linux")]
    return run_keyring(move || {
        Entry::new(SERVICE, &key)
            .ok()
            .and_then(|e| e.get_password().ok())
    }).ok().flatten();
    #[cfg(not(target_os = "linux"))]
    Entry::new(SERVICE, &key)
        .ok()
        .and_then(|e| e.get_password().ok())
}

#[allow(dead_code)]
pub fn delete(host: &str, port: &str) -> Result<(), String> {
    let key = user_key(host, port);
    #[cfg(target_os = "linux")]
    return run_keyring(move || {
        Entry::new(SERVICE, &key)
            .and_then(|e| e.delete_credential())
            .map_err(|e| e.to_string())
    }).and_then(|r| r);
    #[cfg(not(target_os = "linux"))]
    Entry::new(SERVICE, &key)
        .and_then(|e| e.delete_credential())
        .map_err(|e| e.to_string())
}

pub fn save_by_key(key: &str, password: &str) -> Result<(), String> {
    let key = key.to_string();
    let password = password.to_string();
    #[cfg(target_os = "linux")]
    return run_keyring(move || {
        Entry::new(SERVICE, &key)
            .and_then(|e| e.set_password(&password))
            .map_err(|e| e.to_string())
    }).and_then(|r| r);
    #[cfg(not(target_os = "linux"))]
    Entry::new(SERVICE, &key)
        .and_then(|e| e.set_password(&password))
        .map_err(|e| e.to_string())
}

pub fn load_by_key(key: &str) -> Option<String> {
    let key = key.to_string();
    #[cfg(target_os = "linux")]
    return run_keyring(move || {
        Entry::new(SERVICE, &key)
            .ok()
            .and_then(|e| e.get_password().ok())
    }).ok().flatten();
    #[cfg(not(target_os = "linux"))]
    Entry::new(SERVICE, &key)
        .ok()
        .and_then(|e| e.get_password().ok())
}
