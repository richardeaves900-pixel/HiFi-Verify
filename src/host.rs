use std::io::Read;
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static IN_FLATPAK: OnceLock<bool> = OnceLock::new();

pub fn in_flatpak() -> bool {
    *IN_FLATPAK.get_or_init(|| std::path::Path::new("/.flatpak-info").exists())
}

fn build_host_command(args: &[&str]) -> std::io::Result<Command> {
    if args.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty argv",
        ));
    }
    if in_flatpak() {
        let mut c = Command::new("flatpak-spawn");
        c.arg("--host").args(args);
        Ok(c)
    } else {
        let mut c = Command::new(args[0]);
        c.args(&args[1..]);
        Ok(c)
    }
}

/// Run a host command with a real wall-clock timeout and non-deadlocking pipe reads.
pub fn host_cmd(args: &[&str], timeout: Duration) -> std::io::Result<Output> {
    let mut cmd = build_host_command(args)?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stdout {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stderr {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                let now = Instant::now();
                if now >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out_handle.join();
                    let _ = err_handle.join();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!(
                            "command timed out after {}ms: {}",
                            timeout.as_millis(),
                            args[0]
                        ),
                    ));
                }
                let remaining = deadline.saturating_duration_since(now);
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
        }
    };

    let stdout_bytes = out_handle.join().unwrap_or_default();
    let stderr_bytes = err_handle.join().unwrap_or_default();
    Ok(Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    })
}

/// Spawn a long-running streaming command on the host with stdout piped.
pub fn host_spawn_piped(args: &[&str]) -> std::io::Result<Child> {
    let mut cmd = build_host_command(args)?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

/// Spawn a fire-and-forget GUI/desktop application on the host and reap it on a
/// waiter thread so zombie processes do not accumulate.
pub fn host_popen(cmd: &str, args: &[&str]) {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(cmd);
    argv.extend_from_slice(args);

    let Ok(mut command) = build_host_command(&argv) else {
        return;
    };
    if let Ok(mut child) = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Check whether an executable exists on the host PATH.
pub fn command_exists_on_host(bin: &str) -> bool {
    if bin.trim().is_empty() {
        return false;
    }
    host_cmd(
        &["sh", "-c", "command -v \"$1\" >/dev/null 2>&1", "--", bin],
        Duration::from_millis(500),
    )
    .is_ok_and(|o| o.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_argv_returns_invalid_input() {
        let err = host_cmd(&[], Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn host_cmd_enforces_timeout() {
        let start = Instant::now();
        let err = host_cmd(&["sleep", "5"], Duration::from_millis(80)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn host_cmd_captures_stdout() {
        let out = host_cmd(&["echo", "hifi-ok"], Duration::from_secs(1)).unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hifi-ok");
    }
}
