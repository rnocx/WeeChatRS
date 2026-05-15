/// A running SSH local-forward tunnel.
///
/// Spawns `ssh -N -L local_port:relay_host:relay_port user@ssh_host` and
/// kills the process when dropped. If a password is supplied it is delivered
/// via the SSH_ASKPASS mechanism (a temp script that reads the password from
/// an env var — the password itself never touches the script file).
pub struct SshTunnel {
    child: std::process::Child,
    pub local_port: u16,
    /// Temp askpass script kept alive until the tunnel is dropped.
    askpass_path: Option<std::path::PathBuf>,
}

impl SshTunnel {
    pub fn spawn(
        ssh_host: &str,
        ssh_port: Option<u16>,
        ssh_user: &str,
        password: Option<&str>,
        relay_host: &str,
        relay_port: u16,
    ) -> Result<Self, String> {
        let local_port = free_local_port()?;
        let forward = format!("{}:{}:{}", local_port, relay_host, relay_port);
        let target = if ssh_user.is_empty() {
            ssh_host.to_string()
        } else {
            format!("{}@{}", ssh_user, ssh_host)
        };

        let mut cmd = std::process::Command::new("ssh");
        cmd.args([
            "-N",
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", "ExitOnForwardFailure=yes",
            "-o", "ServerAliveInterval=10",
            "-o", "ServerAliveCountMax=3",
            "-L", &forward,
        ]);
        if let Some(port) = ssh_port {
            cmd.args(["-p", &port.to_string()]);
        }
        cmd.arg(&target);

        // Wire up SSH_ASKPASS if a password was provided.
        let askpass_path = if let Some(pw) = password {
            let path = write_askpass_script()?;
            cmd.env("SSH_ASKPASS", &path);
            cmd.env("SSH_ASKPASS_REQUIRE", "force");
            cmd.env("SSH_TUNNEL_PASS", pw);
            // Suppress X11 askpass on Linux with older OpenSSH.
            #[cfg(target_os = "linux")]
            cmd.env("DISPLAY", "");
            Some(path)
        } else {
            None
        };

        // Prevent a console window flashing on Windows.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let child = cmd.spawn().map_err(|e| format!("Failed to launch ssh: {}", e))?;
        Ok(SshTunnel { child, local_port, askpass_path })
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        if let Some(path) = &self.askpass_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Write a tiny askpass script that reads the password from the
/// `SSH_TUNNEL_PASS` environment variable and prints it.
/// The password never appears in the script file itself.
fn write_askpass_script() -> Result<std::path::PathBuf, String> {
    let pid = std::process::id();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(".ssh-askpass-{}", pid));
        std::fs::write(&path, "#!/bin/sh\nprintf '%s' \"$SSH_TUNNEL_PASS\"\n")
            .map_err(|e| format!("Failed to write askpass script: {}", e))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Failed to set askpass permissions: {}", e))?;
        Ok(path)
    }

    #[cfg(windows)]
    {
        // On Windows, SSH_ASKPASS must be an executable. We write a .bat file
        // that prints %SSH_TUNNEL_PASS% and set SSH_ASKPASS to cmd.exe /c <file>.
        // OpenSSH for Windows invokes SSH_ASKPASS directly, so we use a .bat wrapper.
        let path = std::env::temp_dir().join(format!("ssh-askpass-{}.bat", pid));
        std::fs::write(&path, "@echo off\r\necho %SSH_TUNNEL_PASS%\r\n")
            .map_err(|e| format!("Failed to write askpass script: {}", e))?;
        Ok(path)
    }

    #[cfg(not(any(unix, windows)))]
    Err("SSH password authentication is not supported on this platform".to_string())
}

fn free_local_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Could not find a free local port: {}", e))?;
    Ok(l.local_addr()
        .map_err(|e| format!("local_addr failed: {}", e))?
        .port())
}
