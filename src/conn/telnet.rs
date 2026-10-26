//! Telnet client with the option negotiation needed by real servers
//! (ECHO, SGA, TTYPE, NAWS).

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedReceiver;

use super::{ConnCtx, Status};
use crate::model::ResolvedTarget;
use crate::term::Input;

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

#[derive(Default)]
pub struct TelnetParser {
    state: u8,
    cmd: u8,
    sb: Vec<u8>,
}

/// Result of feeding bytes through the parser: data for the terminal and replies for the server.
pub struct Parsed {
    pub data: Vec<u8>,
    pub replies: Vec<u8>,
    pub naws_requested: bool,
}

impl TelnetParser {
    pub fn feed(&mut self, input: &[u8], term_type: &str) -> Parsed {
        let mut out = Parsed {
            data: Vec::with_capacity(input.len()),
            replies: vec![],
            naws_requested: false,
        };
        for &b in input {
            match self.state {
                0 => {
                    if b == IAC {
                        self.state = 1;
                    } else {
                        out.data.push(b);
                    }
                }
                1 => match b {
                    IAC => {
                        out.data.push(IAC);
                        self.state = 0;
                    }
                    DO | DONT | WILL | WONT => {
                        self.cmd = b;
                        self.state = 2;
                    }
                    SB => {
                        self.sb.clear();
                        self.state = 3;
                    }
                    _ => self.state = 0,
                },
                2 => {
                    let opt = b;
                    match self.cmd {
                        DO => match opt {
                            OPT_TTYPE | OPT_SGA => out.replies.extend_from_slice(&[IAC, WILL, opt]),
                            OPT_NAWS => {
                                out.replies.extend_from_slice(&[IAC, WILL, opt]);
                                out.naws_requested = true;
                            }
                            _ => out.replies.extend_from_slice(&[IAC, WONT, opt]),
                        },
                        WILL => match opt {
                            OPT_ECHO | OPT_SGA => out.replies.extend_from_slice(&[IAC, DO, opt]),
                            _ => out.replies.extend_from_slice(&[IAC, DONT, opt]),
                        },
                        _ => {}
                    }
                    self.state = 0;
                }
                3 => {
                    if b == IAC {
                        self.state = 4;
                    } else {
                        self.sb.push(b);
                    }
                }
                _ => {
                    if b == SE {
                        if self.sb.first() == Some(&OPT_TTYPE) && self.sb.get(1) == Some(&1) {
                            out.replies.extend_from_slice(&[IAC, SB, OPT_TTYPE, 0]);
                            out.replies.extend_from_slice(term_type.as_bytes());
                            out.replies.extend_from_slice(&[IAC, SE]);
                        }
                        self.state = 0;
                    } else {
                        self.sb.push(b);
                        self.state = 3;
                    }
                }
            }
        }
        out
    }
}

pub fn naws(cols: u16, rows: u16) -> Vec<u8> {
    let mut v = vec![IAC, SB, OPT_NAWS];
    for b in cols.to_be_bytes().into_iter().chain(rows.to_be_bytes()) {
        v.push(b);
        if b == IAC {
            v.push(IAC);
        }
    }
    v.extend_from_slice(&[IAC, SE]);
    v
}

pub async fn run(t: ResolvedTarget, ctx: ConnCtx, mut rx: UnboundedReceiver<Input>) -> Result<()> {
    ctx.set_status(Status::Connecting(format!("Connecting to {}…", t.address)));
    ctx.info(&format!("→ Telnet {}:{}", t.address, t.port));
    let stream = tokio::net::TcpStream::connect((t.address.as_str(), t.port))
        .await
        .with_context(|| format!("cannot reach {}:{}", t.address, t.port))?;
    let (mut r, mut w) = stream.into_split();
    ctx.set_status(Status::Connected);
    let mut parser = TelnetParser::default();
    let mut size = (100u16, 30u16);
    let mut buf = vec![0u8; 32 * 1024];
    let reason = loop {
        tokio::select! {
            n = r.read(&mut buf) => {
                let n = n.unwrap_or(0);
                if n == 0 { break "Connection closed by remote host".to_string(); }
                let p = parser.feed(&buf[..n], &t.term_type);
                if !p.replies.is_empty() { w.write_all(&p.replies).await?; }
                if p.naws_requested { w.write_all(&naws(size.0, size.1)).await?; }
                if !p.data.is_empty() { ctx.feed(&p.data); }
            }
            input = rx.recv() => match input {
                Some(Input::Data(mut d)) => {
                    // Escape literal IAC bytes.
                    if d.contains(&IAC) {
                        d = d.into_iter().flat_map(|b| if b == IAC { vec![IAC, IAC] } else { vec![b] }).collect();
                    }
                    w.write_all(&d).await?;
                }
                Some(Input::Resize { cols, rows, .. }) => {
                    size = (cols, rows);
                    w.write_all(&naws(cols, rows)).await?;
                }
                Some(Input::Close) | None => break "Session closed".to_string(),
            }
        }
    };
    ctx.info(&format!("\r\n● {reason}"));
    ctx.set_status(Status::Closed(reason));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_and_strips_commands() {
        let mut p = TelnetParser::default();
        let out = p.feed(
            &[b'h', IAC, DO, OPT_NAWS, b'i', IAC, WILL, OPT_ECHO, IAC, IAC],
            "xterm",
        );
        assert_eq!(out.data, vec![b'h', b'i', IAC]);
        assert!(out.naws_requested);
        assert_eq!(out.replies, vec![IAC, WILL, OPT_NAWS, IAC, DO, OPT_ECHO]);
    }

    #[test]
    fn answers_terminal_type_subnegotiation() {
        let mut p = TelnetParser::default();
        let out = p.feed(&[IAC, SB, OPT_TTYPE, 1, IAC, SE], "xterm");
        assert_eq!(
            out.replies,
            [&[IAC, SB, OPT_TTYPE, 0][..], b"xterm", &[IAC, SE]].concat()
        );
    }
}
