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

/// Remove a stored secret; removing one that doesn't exist is not an error.
pub fn delete_by_key(key: &str) -> Result<(), String> {
    let key = key.to_string();
    let delete = move || match Entry::new(SERVICE, &key).and_then(|e| e.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    };
    #[cfg(target_os = "linux")]
    return run_keyring(delete).and_then(|r| r);
    #[cfg(not(target_os = "linux"))]
    delete()
}

/// A stored secret: `Ok(None)` when there is none, `Err` when the keychain couldn't
/// be asked (locked, the prompt refused, no secret service running).
pub fn load_by_key_checked(key: &str) -> Result<Option<String>, String> {
    let key = key.to_string();
    let load = move || match Entry::new(SERVICE, &key).and_then(|e| e.get_password()) {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    };
    #[cfg(target_os = "linux")]
    return run_keyring(load).and_then(|r| r);
    #[cfg(not(target_os = "linux"))]
    load()
}

/// A stored secret, or None when there is none or the keychain couldn't be asked
/// (that is logged). Use `load_by_key_checked` where the user should be told.
pub fn load_by_key(key: &str) -> Option<String> {
    match load_by_key_checked(key) {
        Ok(secret) => secret,
        Err(e) => {
            log::warn!("the keychain couldn't be read: {e}");
            None
        }
    }
}
