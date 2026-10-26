//! Serial console connections.

use std::io::{Read, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::sync::mpsc::UnboundedReceiver;

use super::{ConnCtx, Status};
use crate::model::{FlowControl, Parity, ResolvedTarget};
use crate::term::Input;

pub fn list_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|v| v.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default()
}

pub const BAUD_RATES: &[u32] = &[
    300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 74880, 115_200, 230_400, 460_800, 921_600,
    1_000_000, 1_500_000,
];

pub fn run(t: ResolvedTarget, ctx: ConnCtx, mut rx: UnboundedReceiver<Input>) -> Result<()> {
    let cfg = &t.serial;
    let device = if cfg.device.is_empty() {
        t.address.clone()
    } else {
        cfg.device.clone()
    };
    if device.is_empty() {
        bail!("no serial device selected");
    }
    ctx.info(&format!("→ Opening {device} at {} baud", cfg.baud_rate));
    let mut port = serialport::new(&device, cfg.baud_rate)
        .data_bits(match cfg.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        })
        .stop_bits(if cfg.stop_bits == 2 {
            serialport::StopBits::Two
        } else {
            serialport::StopBits::One
        })
        .parity(match cfg.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        })
        .flow_control(match cfg.flow_control {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::Software => serialport::FlowControl::Software,
            FlowControl::Hardware => serialport::FlowControl::Hardware,
        })
        .timeout(Duration::from_millis(50))
        .open()
        .with_context(|| format!("opening serial port {device}"))?;
    let mut reader = port.try_clone().context("cloning serial port")?;
    ctx.set_status(Status::Connected);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop2 = stop.clone();
    let rctx = ctx.clone();
    let rt = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !stop2.load(std::sync::atomic::Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => rctx.feed(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
        }
    });
    let echo = cfg.local_echo;
    let line_ending = cfg.line_ending.clone();
    while let Some(msg) = rx.blocking_recv() {
        match msg {
            Input::Data(d) => {
                let d: Vec<u8> = if line_ending != "\r" {
                    d.iter()
                        .flat_map(|b| {
                            if *b == b'\r' {
                                line_ending.as_bytes().to_vec()
                            } else {
                                vec![*b]
                            }
                        })
                        .collect()
                } else {
                    d
                };
                if echo {
                    ctx.feed(&d);
                }
                if port.write_all(&d).is_err() {
                    break;
                }
            }
            Input::Resize { .. } => {}
            Input::Close => break,
        }
        if rt.is_finished() {
            break;
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = rt.join();
    ctx.info("\r\n● Serial port closed");
    ctx.set_status(Status::Closed("Serial port closed".into()));
    Ok(())
}
