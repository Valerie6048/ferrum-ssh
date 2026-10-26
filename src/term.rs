//! Terminal emulation state shared between the I/O task (which feeds bytes)
//! and the UI (which renders and sends input).

use std::fs::File;
use std::io::Write;
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;

/// Messages from the UI to a running session.
#[derive(Debug)]
pub enum Input {
    Data(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
        px_w: u16,
        px_h: u16,
    },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Default)]
pub struct TermEvents {
    pub title: Option<String>,
    pub bell: bool,
    pub clipboard: Option<String>,
    pub cell_size: (u16, u16),
    pub size: (u16, u16),
}

#[derive(Clone)]
pub struct Listener {
    pub input: Arc<Mutex<UnboundedSender<Input>>>,
    pub events: Arc<Mutex<TermEvents>>,
    pub ctx: egui::Context,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(s) => {
                let _ = self.input.lock().send(Input::Data(s.into_bytes()));
            }
            Event::Title(t) => self.events.lock().title = Some(t),
            Event::ResetTitle => self.events.lock().title = None,
            Event::Bell => self.events.lock().bell = true,
            Event::ClipboardStore(_, text) => self.events.lock().clipboard = Some(text),
            Event::TextAreaSizeRequest(f) => {
                let ev = self.events.lock();
                let ws = WindowSize {
                    num_lines: ev.size.1,
                    num_cols: ev.size.0,
                    cell_width: ev.cell_size.0,
                    cell_height: ev.cell_size.1,
                };
                drop(ev);
                let _ = self.input.lock().send(Input::Data(f(ws).into_bytes()));
            }
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
            _ => {}
        }
        self.ctx.request_repaint();
    }
}

pub struct TermShared {
    pub term: Term<Listener>,
    processor: Processor,
    pub events: Arc<Mutex<TermEvents>>,
    decoder: Option<encoding_rs::Decoder>,
    log: Option<File>,
    pub log_path: Option<String>,
    pub total_bytes: u64,
    /// Set when new output arrives so the UI can auto-scroll / mark activity.
    pub activity: bool,
}

impl TermShared {
    pub fn new(listener: Listener, scrollback: usize, charset: &str) -> Self {
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let size = TermSize {
            cols: 100,
            rows: 30,
        };
        let events = listener.events.clone();
        let decoder =
            if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("utf8") {
                None
            } else {
                encoding_rs::Encoding::for_label(charset.as_bytes()).map(|e| e.new_decoder())
            };
        Self {
            term: Term::new(config, &size, listener),
            processor: Processor::new(),
            events,
            decoder,
            log: None,
            log_path: None,
            total_bytes: 0,
            activity: false,
        }
    }

    pub fn start_log(&mut self, path: std::path::PathBuf) {
        if let Ok(f) = File::create(&path) {
            self.log = Some(f);
            self.log_path = Some(path.to_string_lossy().to_string());
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.total_bytes += bytes.len() as u64;
        self.activity = true;
        if let Some(log) = self.log.as_mut() {
            let _ = log.write_all(strip_ansi(bytes).as_bytes());
        }
        if let Some(dec) = self.decoder.as_mut() {
            let mut out = String::with_capacity(bytes.len() * 2);
            let _ = dec.decode_to_string(bytes, &mut out, false);
            self.processor.advance(&mut self.term, out.as_bytes());
        } else {
            self.processor.advance(&mut self.term, bytes);
        }
    }

    /// Feed a local status line (e.g. "Connecting…") in dim color.
    pub fn status_line(&mut self, text: &str) {
        let msg = format!("\x1b[2m{text}\x1b[0m\r\n");
        self.processor.advance(&mut self.term, msg.as_bytes());
    }

    pub fn resize(&mut self, cols: usize, rows: usize, cell_w: u16, cell_h: u16) {
        let mut ev = self.events.lock();
        ev.size = (cols as u16, rows as u16);
        ev.cell_size = (cell_w, cell_h);
        drop(ev);
        self.term.resize(TermSize { cols, rows });
    }

    /// Plain text of the visible screen plus scrollback (used for search/export).
    pub fn full_text(&self) -> String {
        let grid = self.term.grid();
        let top = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(-(grid.history_size() as i32)),
            alacritty_terminal::index::Column(0),
        );
        let bottom = alacritty_terminal::index::Point::new(
            alacritty_terminal::index::Line(grid.screen_lines() as i32 - 1),
            alacritty_terminal::index::Column(grid.columns().saturating_sub(1)),
        );
        self.term.bounds_to_string(top, bottom)
    }
}

impl Drop for TermShared {
    fn drop(&mut self) {
        if let Some(mut f) = self.log.take() {
            let _ = f.flush();
        }
    }
}

/// Encode text typed by the user according to the session charset.
pub fn encode_input(charset: &str, text: &str) -> Vec<u8> {
    if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("utf8") {
        return text.as_bytes().to_vec();
    }
    match encoding_rs::Encoding::for_label(charset.as_bytes()) {
        Some(enc) => enc.encode(text).0.into_owned(),
        None => text.as_bytes().to_vec(),
    }
}

/// Remove ANSI/VT escape sequences for human-readable session logs.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\r' => {}
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

pub const CHARSETS: &[&str] = &[
    "UTF-8",
    "ISO-8859-1",
    "ISO-8859-2",
    "ISO-8859-15",
    "Windows-1250",
    "Windows-1251",
    "Windows-1252",
    "KOI8-R",
    "KOI8-U",
    "Shift_JIS",
    "EUC-JP",
    "EUC-KR",
    "GBK",
    "GB18030",
    "Big5",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_color_codes_and_osc() {
        assert_eq!(strip_ansi(b"\x1b[31mred\x1b[0m plain\r\n"), "red plain\n");
        assert_eq!(strip_ansi(b"\x1b]0;title\x07after"), "after");
    }

    #[test]
    fn encodes_latin1() {
        assert_eq!(encode_input("ISO-8859-1", "é"), vec![0xE9]);
        assert_eq!(encode_input("UTF-8", "é"), "é".as_bytes());
    }
}
