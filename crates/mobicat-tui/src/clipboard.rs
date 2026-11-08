//! Copying text: a local clipboard command when one is available, otherwise
//! OSC 52 (works over SSH in most terminals).

use std::io::Write;
use std::process::{Command, Stdio};

use base64::Engine as _;

/// Most terminals cap OSC 52 payloads; stay well under common limits.
const OSC52_MAX: usize = 100_000;

pub fn copy(text: &str) -> Result<&'static str, String> {
    let remote =
        std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CONNECTION").is_some();
    if !remote {
        for (program, args) in commands() {
            if run(program, args, text).is_ok() {
                return Ok(program);
            }
        }
    }
    osc52(text)
}

fn commands() -> Vec<(&'static str, &'static [&'static str])> {
    if cfg!(target_os = "macos") {
        vec![("pbcopy", &[])]
    } else if cfg!(windows) {
        vec![("clip.exe", &[])]
    } else {
        let mut v: Vec<(&'static str, &'static [&'static str])> = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            v.push(("wl-copy", &[]));
        }
        v.push(("xclip", &["-selection", "clipboard"]));
        v.push(("xsel", &["--clipboard", "--input"]));
        v
    }
}

fn run(program: &str, args: &[&str], text: &str) -> std::io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("{program} failed")))
    }
}

fn osc52(text: &str) -> Result<&'static str, String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    if encoded.len() > OSC52_MAX {
        return Err("too large for the terminal clipboard; use Ctrl-s to save instead".into());
    }
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{encoded}\x07")
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())?;
    Ok("OSC 52")
}
