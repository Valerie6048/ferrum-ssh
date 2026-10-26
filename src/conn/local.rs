//! Local shell (and generic local command, used by Mosh) via a cross-platform PTY.

use std::io::{Read, Write};

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::mpsc::UnboundedReceiver;

use super::{ConnCtx, Status};
use crate::term::Input;

pub fn default_shell(configured: &str) -> String {
    if !configured.trim().is_empty() {
        return configured.trim().to_string();
    }
    #[cfg(windows)]
    {
        if which("pwsh.exe") {
            return "pwsh.exe".into();
        }
        "powershell.exe".into()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

#[cfg(windows)]
fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).exists()))
        .unwrap_or(false)
}

/// Available shells for the settings picker.
pub fn available_shells() -> Vec<String> {
    let mut v = vec![];
    #[cfg(windows)]
    {
        for s in [
            "pwsh.exe",
            "powershell.exe",
            "cmd.exe",
            "wsl.exe",
            "bash.exe",
        ] {
            if which(s) {
                v.push(s.to_string());
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(list) = std::fs::read_to_string("/etc/shells") {
            for l in list.lines() {
                let l = l.trim();
                if l.starts_with('/') && std::path::Path::new(l).exists() {
                    v.push(l.to_string());
                }
            }
        }
    }
    v.dedup();
    v
}

/// Run `cmd` (or the user's shell when `None`) inside a PTY.
pub fn run(
    cmd: Option<(String, Vec<String>, Vec<(String, String)>)>,
    ctx: ConnCtx,
    mut rx: UnboundedReceiver<Input>,
) -> Result<()> {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening PTY")?;
    let mut builder = match &cmd {
        Some((prog, args, env)) => {
            let mut b = CommandBuilder::new(prog);
            b.args(args);
            for (k, v) in env {
                b.env(k, v);
            }
            b
        }
        None => {
            let shell = default_shell(&ctx.local_shell);
            let mut parts = shell_words::split(&shell).unwrap_or_else(|_| vec![shell.clone()]);
            let prog = parts.remove(0);
            let mut b = CommandBuilder::new(prog);
            b.args(parts);
            #[cfg(not(windows))]
            if b.get_argv().len() == 1 {
                b.arg("-l");
            }
            b
        }
    };
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("TERM_PROGRAM", "Ferrum");
    if let Some(home) = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()) {
        builder.cwd(home);
    }
    let mut child = pair
        .slave
        .spawn_command(builder)
        .context("starting shell")?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let master = pair.master;
    ctx.set_status(Status::Connected);

    let rctx = ctx.clone();
    let reader_thread = std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => rctx.feed(&buf[..n]),
            }
        }
    });

    while let Some(msg) = rx.blocking_recv() {
        match msg {
            Input::Data(d) => {
                if writer.write_all(&d).is_err() {
                    break;
                }
                let _ = writer.flush();
            }
            Input::Resize {
                cols,
                rows,
                px_w,
                px_h,
            } => {
                let _ = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: px_w,
                    pixel_height: px_h,
                });
            }
            Input::Close => {
                let _ = child.kill();
                break;
            }
        }
        if reader_thread.is_finished() {
            break;
        }
    }
    let code = child.wait().map(|s| s.exit_code()).unwrap_or(0);
    let _ = reader_thread.join();
    let reason = format!("Process exited with code {code}");
    ctx.info(&format!("\r\n● {reason}"));
    ctx.set_status(Status::Closed(reason));
    Ok(())
}
