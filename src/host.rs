use std::process::{Command, Output};
use std::time::Duration;

pub fn host_cmd(args: &[&str], _timeout: Duration) -> std::io::Result<Output> {
    if args.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty argv",
        ));
    }
    if std::path::Path::new("/.flatpak-info").exists() {
        let mut c = Command::new("flatpak-spawn");
        c.arg("--host").args(args);
        c.output()
    } else {
        Command::new(args[0]).args(&args[1..]).output()
    }
}

pub fn host_popen(cmd: &str, args: &[&str]) {
    let in_flatpak = std::path::Path::new("/.flatpak-info").exists();
    let mut c = if in_flatpak {
        let mut c = std::process::Command::new("flatpak-spawn");
        c.arg("--host").arg(cmd);
        c
    } else {
        std::process::Command::new(cmd)
    };

    let _ = c
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
