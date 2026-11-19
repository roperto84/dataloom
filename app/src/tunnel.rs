//! Local SSH port forwarding for database connections.
//!
//! The app shells out to the system `ssh` binary instead of embedding a second
//! SSH stack. That keeps the user's `~/.ssh/config`, agent and hardware keys
//! working, at the cost of requiring key/agent authentication: `BatchMode=yes`
//! means ssh never opens an interactive password prompt.

use crate::db::SavedConnection;
use std::fs::{self, File};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A live `ssh -L` process. Dropping it closes the tunnel.
pub struct SshTunnel {
    child: Child,
    log_path: PathBuf,
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.log_path);
    }
}

/// Starts a tunnel and returns the connection parameters to use against it.
/// The original connection keeps the real database host for display and TLS
/// policy; the returned copy points at 127.0.0.1 and the local forwarded port.
pub fn start(saved: &SavedConnection) -> Result<(SshTunnel, SavedConnection), String> {
    let ssh_host = saved.ssh_host.trim();
    if ssh_host.is_empty() {
        return Err("This connection has no SSH host.".to_string());
    }
    let db_host = if saved.host.trim().is_empty() {
        "127.0.0.1"
    } else {
        saved.host.trim()
    };
    let db_port = if saved.port == 0 {
        saved.driver_kind().default_port()
    } else {
        saved.port
    };
    let local_port = free_local_port()?;
    let log_path = std::env::temp_dir().join(format!(
        "dataloom-ssh-{}-{local_port}.log",
        std::process::id()
    ));
    let log = File::create(&log_path)
        .map_err(|error| format!("Could not create the SSH log: {error}"))?;

    let mut command = ssh_command(saved, local_port, db_host, db_port);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start the SSH client: {error}"))?;

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, local_port));
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Could not check the SSH client: {error}"))?
        {
            let details = read_log(&log_path);
            let _ = fs::remove_file(&log_path);
            return Err(format!(
                "The SSH tunnel exited before it was ready ({status}).{details}"
            ));
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let details = read_log(&log_path);
            let _ = fs::remove_file(&log_path);
            return Err(format!(
                "The SSH tunnel did not become ready within 12 seconds.{details}"
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut effective = saved.clone();
    effective.host = "127.0.0.1".to_string();
    effective.port = local_port;
    Ok((
        SshTunnel { child, log_path },
        effective,
    ))
}

fn ssh_command(
    saved: &SavedConnection,
    local_port: u16,
    db_host: &str,
    db_port: u16,
) -> Command {
    let ssh_port = if saved.ssh_port == 0 { 22 } else { saved.ssh_port };
    let mut command = Command::new("/usr/bin/ssh");
    command
        .arg("-N")
        .arg("-T")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-p")
        .arg(ssh_port.to_string())
        .arg("-L")
        .arg(format!(
            "127.0.0.1:{local_port}:{}:{db_port}",
            if db_host.contains(':') {
                format!("[{db_host}]")
            } else {
                db_host.to_string()
            }
        ));
    if !saved.ssh_key_path.trim().is_empty() {
        command.arg("-i").arg(expand_tilde(saved.ssh_key_path.trim()));
    }
    let host = saved.ssh_host.trim();
    let user = saved.ssh_user.trim();
    if user.is_empty() {
        command.arg(host);
    } else {
        command.arg(format!("{user}@{host}"));
    }
    command
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/{rest}")
    } else {
        path.to_string()
    }
}

fn free_local_port() -> Result<u16, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("Could not reserve a local port for SSH: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("Could not read the local SSH port: {error}"))?
        .port();
    drop(listener);
    Ok(port)
}

fn read_log(path: &std::path::Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let tail: String = text
        .chars()
        .rev()
        .take(500)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!(" ssh said: {tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_saved() -> SavedConnection {
        SavedConnection {
            id: 1,
            name: "Tunnel".to_string(),
            path: String::new(),
            driver: "PostgreSQL".to_string(),
            host: "db.internal".to_string(),
            port: 5432,
            user: "app".to_string(),
            password: String::new(),
            database: "shop".to_string(),
            read_only: false,
            ssl_mode: "disable".to_string(),
            ssl_root_cert: String::new(),
            ssh_host: "bastion.example.com".to_string(),
            ssh_port: 2222,
            ssh_user: "deploy".to_string(),
            ssh_key_path: "~/.ssh/id_ed25519".to_string(),
            group_id: None,
        }
    }

    #[test]
    fn ssh_command_forwards_the_database_and_uses_batch_mode() {
        let command = ssh_command(&ssh_saved(), 4242, "db.internal", 5432);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(args.iter().any(|arg| arg == "-N"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-L" && pair[1] == "127.0.0.1:4242:db.internal:5432"));
        assert!(args.iter().any(|arg| arg == "BatchMode=yes"));
        assert!(args.windows(2).any(|pair| pair[0] == "-p" && pair[1] == "2222"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-i" && pair[1].ends_with(".ssh/id_ed25519")));
        assert_eq!(args.last().map(String::as_str), Some("deploy@bastion.example.com"));
    }
}
