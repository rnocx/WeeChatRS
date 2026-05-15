/// A running SSH local-forward tunnel.
///
/// Spawns `ssh -N -L local_port:relay_host:relay_port user@ssh_host` and
/// kills the process when dropped.
pub struct SshTunnel {
    child: std::process::Child,
    pub local_port: u16,
}

impl SshTunnel {
    pub fn spawn(
        ssh_host: &str,
        ssh_port: Option<u16>,
        ssh_user: &str,
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

        // Prevent a console window flashing on Windows.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        let child = cmd.spawn().map_err(|e| format!("Failed to launch ssh: {}", e))?;
        Ok(SshTunnel { child, local_port })
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn free_local_port() -> Result<u16, String> {
    // Bind to port 0 — the OS picks a free port — then release it.
    // There is a brief TOCTOU window, but it is acceptable here.
    let l = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Could not find a free local port: {}", e))?;
    Ok(l.local_addr()
        .map_err(|e| format!("local_addr failed: {}", e))?
        .port())
}
