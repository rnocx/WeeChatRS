use keyring::Entry;

const SERVICE: &str = "weechat-rs";

#[allow(dead_code)]
fn user_key(host: &str, port: &str) -> String {
    format!("{}:{}", host, port)
}

#[allow(dead_code)]
pub fn save(host: &str, port: &str, password: &str) -> Result<(), String> {
    Entry::new(SERVICE, &user_key(host, port))
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

#[allow(dead_code)]
pub fn load(host: &str, port: &str) -> Option<String> {
    Entry::new(SERVICE, &user_key(host, port))
        .ok()
        .and_then(|e| e.get_password().ok())
}

#[allow(dead_code)]
pub fn delete(host: &str, port: &str) -> Result<(), String> {
    Entry::new(SERVICE, &user_key(host, port))
        .and_then(|e| e.delete_credential())
        .map_err(|e| e.to_string())
}

pub fn save_by_key(key: &str, password: &str) -> Result<(), String> {
    Entry::new(SERVICE, key)
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

pub fn load_by_key(key: &str) -> Option<String> {
    Entry::new(SERVICE, key)
        .ok()
        .and_then(|e| e.get_password().ok())
}
