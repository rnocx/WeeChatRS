//! SSH tunnels: `ssh -N -L <local port>:<relay host>:<relay port> <server>` run in the
//! background, so a relay or IRC server is reached through an SSH server.
//!
//! It's the system's OpenSSH, so the SSH agent, `~/.ssh` keys and `~/.ssh/config`
//! (aliases, `IdentityFile`, `ProxyJump`…) all apply. ssh has no terminal here, so
//! when it needs a key's passphrase or a password it asks its SSH_ASKPASS program:
//! this app's own binary (see [`askpass_main`]), which answers from environment
//! variables, so the secrets are never written to a file.
//!
//! A supervisor thread keeps the tunnel up: it logs what ssh says, restarts it on the
//! same local port after a drop, and gives up on errors a retry can't fix.

use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How ssh logs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SshAuth {
    /// The SSH agent, `~/.ssh` keys and `~/.ssh/config`.
    #[default]
    Auto,
    /// One key file, with the passphrase if it has one.
    KeyFile,
    Password,
}

/// Set in ssh's environment, so this binary knows it was started as its askpass.
const ASKPASS_FLAG: &str = "WEECHATRS_SSH_ASKPASS";
const ASKPASS_PASSWORD: &str = "WEECHATRS_SSH_PASSWORD";
const ASKPASS_PASSPHRASE: &str = "WEECHATRS_SSH_PASSPHRASE";

/// When ssh started this binary as its SSH_ASKPASS: answers the prompt in the first
/// argument and returns the exit code. `None` for a normal start.
pub fn askpass_main() -> Option<i32> {
    std::env::var_os(ASKPASS_FLAG)?;
    let prompt = std::env::args().nth(1).unwrap_or_default();
    Some(match askpass_answer(&prompt, |name| std::env::var(name).ok()) {
        Some(answer) => {
            println!("{answer}");
            0
        }
        None => 1,
    })
}

/// The answer to one of ssh's prompts: the passphrase for "Enter passphrase for
/// key…", the password for "password:". Anything else, such as whether to trust a
/// host key, gets no answer, so ssh stops instead of guessing.
fn askpass_answer(prompt: &str, var: impl Fn(&str) -> Option<String>) -> Option<String> {
    let lower = prompt.to_lowercase();
    let name = if lower.contains("passphrase") {
        ASKPASS_PASSPHRASE
    } else if lower.contains("password") {
        ASKPASS_PASSWORD
    } else {
        return None;
    };
    var(name).filter(|value| !value.is_empty())
}

/// An SSH server as typed: `[user@]host[:port]`, `[user@][ipv6]:port`, or a
/// `~/.ssh/config` alias. Returns what ssh connects to, and the port if one was given.
pub fn parse_server(server: &str) -> Result<(String, Option<u16>), String> {
    let server = server.trim();
    if server.is_empty() {
        return Err("No SSH server set".to_string());
    }
    if server.contains(char::is_whitespace) || server.starts_with('-') {
        return Err(format!("Not an SSH server: {server}"));
    }
    let (user, host) = match server.rsplit_once('@') {
        Some((user, host)) if !user.is_empty() => (Some(user), host),
        Some(_) => return Err(format!("Not an SSH server: {server}")),
        None => (None, server),
    };
    let (host, port) = if let Some(rest) = host.strip_prefix('[') {
        let (addr, after) = rest.split_once(']').ok_or_else(|| format!("Not an SSH server: {server}"))?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p.parse::<u16>().map_err(|_| format!("Not a port: {p}"))?),
            None if after.is_empty() => None,
            None => return Err(format!("Not an SSH server: {server}")),
        };
        (addr.to_string(), port)
    } else if host.matches(':').count() == 1 {
        let (h, p) = host.split_once(':').expect("one colon");
        (h.to_string(), Some(p.parse::<u16>().map_err(|_| format!("Not a port: {p}"))?))
    } else {
        // No port, or a bare IPv6 address.
        (host.to_string(), None)
    };
    if host.is_empty() {
        return Err(format!("Not an SSH server: {server}"));
    }
    Ok((user.map_or(host.clone(), |u| format!("{u}@{host}")), port))
}

/// The SSH server field from the separate host, user and port of older settings.
pub fn join_server(host: &str, user: &str, port: Option<u16>) -> String {
    let host = host.trim();
    if host.is_empty() {
        return String::new();
    }
    let mut server = if user.trim().is_empty() || host.contains('@') {
        host.to_string()
    } else {
        format!("{}@{host}", user.trim())
    };
    if let Some(port) = port.filter(|&p| p != 22) {
        let bare = server.rsplit_once('@').map_or(server.as_str(), |(_, h)| h);
        if !bare.contains(':') {
            server.push_str(&format!(":{port}"));
        } else if !bare.starts_with('[') {
            // A bare IPv6 address takes brackets before a port.
            let user = server.rsplit_once('@').map(|(u, _)| format!("{u}@")).unwrap_or_default();
            server = format!("{user}[{bare}]:{port}");
        }
    }
    server
}

/// A key file path with a leading `~/` made absolute.
pub fn expand_home(path: &str) -> std::path::PathBuf {
    let path = path.trim();
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

/// Whether a private key file is protected by a passphrase: an OpenSSH key whose
/// cipher isn't "none", or an encrypted PEM key.
pub fn key_has_passphrase(path: &std::path::Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Can't read the key file: {e}"))?;
    if text.contains("ENCRYPTED") {
        return Ok(true);
    }
    let Some(body) = text.split("-----BEGIN OPENSSH PRIVATE KEY-----").nth(1)
        .and_then(|rest| rest.split("-----END OPENSSH PRIVATE KEY-----").next())
    else {
        // An unencrypted PEM key, or something ssh will report on.
        return Ok(false);
    };
    use base64::Engine;
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let data = base64::engine::general_purpose::STANDARD.decode(compact)
        .map_err(|_| "The key file is damaged".to_string())?;
    let rest = data.strip_prefix(b"openssh-key-v1\0").ok_or("Not an OpenSSH key")?;
    let len = rest.get(..4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or("The key file is damaged")?;
    let cipher = rest.get(4..4 + len).ok_or("The key file is damaged")?;
    Ok(cipher != b"none")
}

/// Everything needed to (re)start a tunnel.
#[derive(Clone)]
pub struct TunnelSpec {
    pub server: String,
    pub auth: SshAuth,
    pub key_file: String,
    /// The password, or the key's passphrase, by `auth`.
    pub secret: Option<String>,
    pub relay_host: String,
    pub relay_port: u16,
}

impl TunnelSpec {
    fn command(&self, local_port: u16) -> Result<std::process::Command, String> {
        let (target, port) = parse_server(&self.server)?;
        let exe = askpass_exe().map_err(|e| format!("Can't find this app's program for ssh's prompts: {e}"))?;
        let mut cmd = std::process::Command::new("ssh");
        cmd.args([
            "-N",
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", "ExitOnForwardFailure=yes",
            "-o", "ServerAliveInterval=10",
            "-o", "ServerAliveCountMax=3",
            "-o", "ConnectTimeout=15",
            // The answer won't change, so trying it again only adds failed logins.
            "-o", "NumberOfPasswordPrompts=1",
            "-L", &format!("127.0.0.1:{local_port}:{}:{}", self.relay_host, self.relay_port),
        ]);
        if let Some(port) = port {
            cmd.args(["-p", &port.to_string()]);
        }
        // Tests keep their host keys out of the user's known_hosts.
        #[cfg(test)]
        if let Some(file) = std::env::var_os("WEECHATRS_TEST_KNOWN_HOSTS") {
            cmd.arg("-o").arg(format!("UserKnownHostsFile={}", file.to_string_lossy()));
        }
        if self.auth == SshAuth::KeyFile {
            let key = expand_home(&self.key_file);
            if !key.is_file() {
                return Err(format!("No key file at {}", key.display()));
            }
            cmd.arg("-i").arg(key).args(["-o", "IdentitiesOnly=yes"]);
        }
        cmd.arg("--").arg(target);

        // ssh asks this app for secrets; with nothing to answer it fails at once
        // instead of waiting on a terminal that isn't there.
        cmd.env("SSH_ASKPASS", exe)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(ASKPASS_FLAG, "1")
            .env_remove(ASKPASS_PASSWORD)
            .env_remove(ASKPASS_PASSPHRASE);
        // Older OpenSSH, without SSH_ASKPASS_REQUIRE, asks only with a display set.
        #[cfg(unix)]
        if std::env::var_os("DISPLAY").is_none() {
            cmd.env("DISPLAY", ":0");
        }
        match (&self.secret, self.auth) {
            (Some(secret), SshAuth::Password) => { cmd.env(ASKPASS_PASSWORD, secret); }
            (Some(secret), SshAuth::KeyFile) => { cmd.env(ASKPASS_PASSPHRASE, secret); }
            _ => {}
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        // No console window flashing up on Windows.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        Ok(cmd)
    }
}

/// The program ssh runs for its prompts: this app. Tests run as another program, so
/// they name the app's binary.
fn askpass_exe() -> std::io::Result<std::path::PathBuf> {
    #[cfg(test)]
    if let Some(exe) = std::env::var_os("WEECHATRS_TEST_ASKPASS_EXE") {
        return Ok(exe.into());
    }
    std::env::current_exe()
}

/// ssh errors a restart won't fix.
fn is_fatal(line: &str) -> bool {
    ["Permission denied", "Host key verification failed", "IDENTIFICATION HAS CHANGED", "Load key", "Bad configuration", "Could not request local forwarding"]
        .iter()
        .any(|s| line.contains(s))
}

/// The line of ssh's output that says most about why it stopped: one that a restart
/// won't fix, else the last that isn't a note about a newly trusted host.
fn reason(lines: &[String]) -> String {
    lines.iter().rev().find(|l| is_fatal(l))
        .or_else(|| lines.iter().rev().find(|l| !l.starts_with("Warning: Permanently added")))
        .cloned()
        .unwrap_or_default()
}

/// What the tunnel reports: lines for the connection log, and a failure that ends it.
pub enum TunnelEvent {
    Log(String),
    Failed(String),
}

/// A running tunnel; dropping it stops ssh.
pub struct SshTunnel {
    pub local_port: u16,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<std::process::Child>>>,
}

impl SshTunnel {
    /// Starts ssh and a thread that keeps it running. `report` gets ssh's messages.
    pub fn spawn(spec: TunnelSpec, report: impl Fn(TunnelEvent) + Send + 'static) -> Result<Self, String> {
        let local_port = free_local_port()?;
        // Fail now on what a restart can't change: ssh missing, a bad server or key.
        let first = spec.command(local_port)?
            .spawn()
            .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound {
                "ssh isn't installed (OpenSSH client)".to_string()
            } else {
                format!("Couldn't start ssh: {e}")
            })?;
        let stop = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let tunnel = SshTunnel { local_port, stop: stop.clone(), child: child.clone() };
        std::thread::spawn(move || supervise(spec, local_port, first, stop, child, report));
        Ok(tunnel)
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

fn supervise(
    spec: TunnelSpec,
    local_port: u16,
    mut process: std::process::Child,
    stop: Arc<AtomicBool>,
    slot: Arc<Mutex<Option<std::process::Child>>>,
    report: impl Fn(TunnelEvent),
) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let started = Instant::now();
        let stderr = process.stderr.take();
        if let Ok(mut s) = slot.lock() {
            *s = Some(process);
        }
        // Stopped while it was starting: Drop found no child to kill.
        if stop.load(Ordering::SeqCst) {
            if let Ok(mut s) = slot.lock() {
                if let Some(c) = s.as_mut() { let _ = c.kill(); }
            }
        }
        // ssh -N writes only warnings and errors; they end when it exits.
        let mut lines = Vec::new();
        if let Some(stderr) = stderr {
            for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                let line = line.trim().to_string();
                if !line.is_empty() {
                    report(TunnelEvent::Log(format!("ssh: {line}")));
                    lines.push(line);
                }
            }
        }
        let last_error = reason(&lines);
        if let Ok(mut s) = slot.lock() {
            if let Some(mut c) = s.take() {
                let _ = c.wait();
            }
        }
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if is_fatal(&last_error) {
            report(TunnelEvent::Failed(format!("SSH tunnel failed: {last_error}")));
            return;
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(2);
        }
        report(TunnelEvent::Log(format!("SSH tunnel closed; restarting in {} s", backoff.as_secs())));
        let until = Instant::now() + backoff;
        while Instant::now() < until {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        backoff = (backoff * 2).min(Duration::from_secs(60));
        process = match spec.command(local_port).and_then(|mut c| c.spawn().map_err(|e| e.to_string())) {
            Ok(p) => p,
            Err(e) => {
                report(TunnelEvent::Failed(format!("SSH tunnel failed to restart: {e}")));
                return;
            }
        };
    }
}

/// Logs in and opens the forward, then checks that the SSH server reaches the relay.
/// Blocking: run it off the UI thread.
pub fn test(spec: &TunnelSpec) -> Result<String, String> {
    let local_port = free_local_port()?;
    let mut child = spec.command(local_port)?.spawn().map_err(|e| format!("Couldn't start ssh: {e}"))?;
    let stderr = child.stderr.take();
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let reader = {
        let lines = lines.clone();
        std::thread::spawn(move || {
            if let Some(stderr) = stderr {
                for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Ok(mut l) = lines.lock() { l.push(line); }
                }
            }
        })
    };
    let last_line = |lines: &Arc<Mutex<Vec<String>>>| lines.lock().map(|l| reason(&l)).unwrap_or_default();

    let deadline = Instant::now() + Duration::from_secs(20);
    let result = loop {
        if let Ok(Some(_)) = child.try_wait() {
            let _ = reader.join();
            let line = last_line(&lines);
            break Err(if line.is_empty() { "ssh stopped without saying why".to_string() } else { line });
        }
        // ssh listens on the local port once it's logged in and forwarding.
        if std::net::TcpListener::bind(("127.0.0.1", local_port)).is_err() {
            // A connection through the tunnel shows whether the SSH server reaches the relay.
            let probe = std::net::TcpStream::connect(("127.0.0.1", local_port));
            std::thread::sleep(Duration::from_millis(1500));
            drop(probe);
            let refused = lines.lock().ok().and_then(|l| l.iter().find(|l| l.contains("open failed")).cloned());
            break match refused {
                // "channel 2: open failed: connect failed: Connection refused"
                Some(line) if line.contains("administratively prohibited") => Err(
                    "Logged in, but the SSH server doesn't allow port forwarding (AllowTcpForwarding in its sshd_config)".to_string(),
                ),
                Some(line) => Err(format!(
                    "Logged in, but the SSH server can't reach {}:{} ({}). The relay host is as seen from the SSH server, often localhost.",
                    spec.relay_host, spec.relay_port,
                    line.split_once("open failed: ").map_or(line.as_str(), |(_, why)| why).trim_start_matches("connect failed: "),
                )),
                None => Ok(format!("Logged in, and the SSH server reaches {}:{}", spec.relay_host, spec.relay_port)),
            };
        }
        if Instant::now() >= deadline {
            break Err("No answer within 20 s".to_string());
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn free_local_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Could not find a free local port: {}", e))?;
    Ok(l.local_addr()
        .map_err(|e| format!("local_addr failed: {}", e))?
        .port())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a real SSH server: set SSH_TEST_SERVER (user@host:port), SSH_TEST_PASSWORD,
    /// SSH_TEST_KEY and SSH_TEST_PASSPHRASE (a key with a passphrase the server accepts),
    /// WEECHATRS_TEST_ASKPASS_EXE (the built weechat-rs) and WEECHATRS_TEST_KNOWN_HOSTS.
    #[test]
    #[ignore]
    fn live_logins_and_restart() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
        let spec = |auth, key: &str, secret: &str, relay_port| TunnelSpec {
            server: env("SSH_TEST_SERVER"),
            auth,
            key_file: key.to_string(),
            secret: Some(secret.to_string()).filter(|s| !s.is_empty()),
            relay_host: "127.0.0.1".into(),
            relay_port,
        };
        let key = env("SSH_TEST_KEY");
        let ok = |s: TunnelSpec| test(&s).unwrap_or_else(|e| panic!("{e}"));
        println!("password: {}", ok(spec(SshAuth::Password, "", &env("SSH_TEST_PASSWORD"), 22)));
        println!("key: {}", ok(spec(SshAuth::KeyFile, &key, &env("SSH_TEST_PASSPHRASE"), 22)));
        let wrong = test(&spec(SshAuth::Password, "", "wrong", 22)).unwrap_err();
        println!("wrong password: {wrong}");
        assert!(wrong.contains("Permission denied"));
        let no_phrase = test(&spec(SshAuth::KeyFile, &key, "", 22)).unwrap_err();
        println!("key without passphrase: {no_phrase}");
        let unreachable = test(&spec(SshAuth::Password, "", &env("SSH_TEST_PASSWORD"), 9)).unwrap_err();
        println!("relay down: {unreachable}");
        assert!(unreachable.contains("can't reach"));

        // A running tunnel comes back after ssh is killed, on the same port.
        let (tx, rx) = std::sync::mpsc::channel();
        let tunnel = SshTunnel::spawn(spec(SshAuth::Password, "", &env("SSH_TEST_PASSWORD"), 22), move |e| {
            let _ = tx.send(match e { TunnelEvent::Log(t) => t, TunnelEvent::Failed(t) => format!("FAILED {t}") });
        }).unwrap();
        let port = tunnel.local_port;
        let listening = || std::net::TcpListener::bind(("127.0.0.1", port)).is_err();
        let wait_for = |what: &dyn Fn() -> bool| {
            let until = Instant::now() + Duration::from_secs(20);
            while !what() { assert!(Instant::now() < until, "timed out"); std::thread::sleep(Duration::from_millis(200)); }
        };
        wait_for(&listening);
        if let Ok(mut c) = tunnel.child.lock() { c.as_mut().unwrap().kill().unwrap(); }
        wait_for(&|| !listening());
        wait_for(&listening);
        let log: Vec<String> = rx.try_iter().collect();
        println!("tunnel log: {log:?}");
        assert!(log.iter().any(|l| l.contains("restarting")));
        drop(tunnel);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!listening(), "ssh still running after drop");
    }

    #[test]
    fn askpass_answers_only_what_it_was_given() {
        let vars = |name: &str| match name {
            ASKPASS_PASSWORD => Some("pw".to_string()),
            ASKPASS_PASSPHRASE => Some("phrase".to_string()),
            _ => None,
        };
        assert_eq!(askpass_answer("Enter passphrase for key '/home/u/.ssh/id_ed25519': ", vars).as_deref(), Some("phrase"));
        assert_eq!(askpass_answer("(bob@host) Password: ", vars).as_deref(), Some("pw"));
        assert_eq!(askpass_answer("bob@host's password: ", vars).as_deref(), Some("pw"));
        assert_eq!(askpass_answer("Are you sure you want to continue connecting (yes/no/[fingerprint])? ", vars), None);
        assert_eq!(askpass_answer("Password: ", |_| Some(String::new())), None);
        assert_eq!(askpass_answer("Password: ", |_| None), None);
    }

    #[test]
    fn servers_parse_with_user_and_port() {
        assert_eq!(parse_server("host").unwrap(), ("host".into(), None));
        assert_eq!(parse_server(" bob@host:2222 ").unwrap(), ("bob@host".into(), Some(2222)));
        assert_eq!(parse_server("bob@[::1]:22").unwrap(), ("bob@::1".into(), Some(22)));
        assert_eq!(parse_server("fe80::1").unwrap(), ("fe80::1".into(), None));
        assert_eq!(parse_server("myalias").unwrap(), ("myalias".into(), None));
        for bad in ["", "two words", "-oProxyCommand=x", "@host", "host:port", "bob@"] {
            assert!(parse_server(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn older_settings_join_into_one_server() {
        assert_eq!(join_server("host", "bob", Some(2222)), "bob@host:2222");
        assert_eq!(join_server("host", "", Some(22)), "host");
        assert_eq!(join_server("alice@host", "bob", None), "alice@host");
        assert_eq!(join_server("::1", "bob", Some(2200)), "bob@[::1]:2200");
        assert_eq!(join_server("", "bob", Some(2200)), "");
        for joined in ["bob@host:2222", "bob@[::1]:2200"] {
            assert!(parse_server(joined).is_ok());
        }
    }

    #[test]
    fn tells_keys_with_a_passphrase() {
        let dir = std::env::temp_dir().join(format!("weechatrs-keytest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ssh_keygen = |name: &str, pass: &str| -> Option<std::path::PathBuf> {
            let path = dir.join(name);
            let _ = std::fs::remove_file(&path);
            let ok = std::process::Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", pass, "-f"]).arg(&path)
                .status().ok()?.success();
            ok.then_some(path)
        };
        if let (Some(plain), Some(locked)) = (ssh_keygen("plain", ""), ssh_keygen("locked", "secret")) {
            assert!(!key_has_passphrase(&plain).unwrap());
            assert!(key_has_passphrase(&locked).unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(key_has_passphrase(std::path::Path::new("/nonexistent/key")).is_err());
    }
}
