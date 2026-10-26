//! Mosh: bootstrap `mosh-server` over SSH, then run the local `mosh-client`
//! in a PTY with the negotiated port and session key.

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc::UnboundedReceiver;

use super::{ConnCtx, Status, local, ssh};
use crate::model::ResolvedTarget;
use crate::term::Input;

/// Parse `MOSH CONNECT <port> <key>` from mosh-server output.
pub fn parse_connect(out: &str) -> Option<(u16, String)> {
    out.lines().find_map(|l| {
        let mut it = l.trim().split_whitespace();
        if it.next()? == "MOSH" && it.next()? == "CONNECT" {
            let port = it.next()?.parse().ok()?;
            let key = it.next()?.to_string();
            Some((port, key))
        } else {
            None
        }
    })
}

pub async fn run(
    t: ResolvedTarget,
    ctx: ConnCtx,
    rx: UnboundedReceiver<Input>,
    _rt: tokio::runtime::Handle,
) -> Result<()> {
    let client = ssh::connect(&t, &ctx).await?;
    ctx.info("→ Starting mosh-server…");
    let (out, _) = client
        .exec(&t.mosh.server_command)
        .await
        .context("running mosh-server")?;
    let (port, key) =
        parse_connect(&out).ok_or_else(|| anyhow!("mosh-server did not start: {}", out.trim()))?;
    client.disconnect().await;
    let ip = tokio::net::lookup_host((t.address.as_str(), 0))
        .await?
        .next()
        .map(|a| a.ip().to_string())
        .ok_or_else(|| anyhow!("cannot resolve {}", t.address))?;
    let prog = if ctx.mosh_client.trim().is_empty() {
        "mosh-client".to_string()
    } else {
        ctx.mosh_client.clone()
    };
    if which(&prog).is_none() {
        bail!(
            "`{prog}` was not found. Install mosh (https://mosh.org) or set its path in Settings → Terminal."
        );
    }
    ctx.info(&format!("→ mosh-client {ip}:{port}"));
    let env = vec![
        ("MOSH_KEY".to_string(), key),
        ("LANG".to_string(), "en_US.UTF-8".to_string()),
    ];
    let ctx2 = ctx.clone();
    tokio::task::spawn_blocking(move || {
        local::run(Some((prog, vec![ip, port.to_string()], env)), ctx2, rx)
    })
    .await
    .map_err(|e| anyhow!("{e}"))??;
    let _ = Status::Connected;
    Ok(())
}

fn which(prog: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(prog);
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    let mut dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin"]
            .iter()
            .map(std::path::PathBuf::from),
    );
    for d in dirs {
        for name in [prog.to_string(), format!("{prog}.exe")] {
            let c = d.join(&name);
            if c.exists() {
                return Some(c);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_mosh_connect_line() {
        let out = "\r\nMOSH CONNECT 60001 4NeCCgvZFe2RnPgrcU1PQw\r\n\r\nmosh-server (mosh 1.4.0)";
        assert_eq!(
            super::parse_connect(out),
            Some((60001, "4NeCCgvZFe2RnPgrcU1PQw".into()))
        );
        assert_eq!(super::parse_connect("nope"), None);
    }
}
