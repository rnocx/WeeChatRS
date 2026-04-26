#[cfg(not(target_os = "linux"))]
use keyring::Entry;

#[cfg(not(target_os = "linux"))]
const SERVICE: &str = "weechat-rs";

#[cfg(not(target_os = "linux"))]
fn user_key(host: &str, port: &str) -> String {
    format!("{}:{}", host, port)
}

#[cfg(not(target_os = "linux"))]
pub fn save(host: &str, port: &str, password: &str) -> Result<(), String> {
    Entry::new(SERVICE, &user_key(host, port))
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "linux"))]
pub fn load(host: &str, port: &str) -> Option<String> {
    Entry::new(SERVICE, &user_key(host, port))
        .ok()
        .and_then(|e| e.get_password().ok())
}

#[cfg(not(target_os = "linux"))]
pub fn delete(host: &str, port: &str) -> Result<(), String> {
    Entry::new(SERVICE, &user_key(host, port))
        .and_then(|e| e.delete_credential())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
pub fn save(_host: &str, _port: &str, _password: &str) -> Result<(), String> { Ok(()) }

#[cfg(target_os = "linux")]
pub fn load(_host: &str, _port: &str) -> Option<String> { None }

#[cfg(target_os = "linux")]
pub fn delete(_host: &str, _port: &str) -> Result<(), String> { Ok(()) }
