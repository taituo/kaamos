//! Terminal control surface. Talks to /dev/tty directly (so stdout stays free
//! for --stdout audio) and uses `stty` for raw keys — no extra crates.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::params::*;
use crate::LiveInfo;

static SAVED: OnceLock<String> = OnceLock::new();

const RED: &str = "\x1b[38;5;203m";
const HOT: &str = "\x1b[1;38;5;197m";
const DIM: &str = "\x1b[38;5;95m";
const TXT: &str = "\x1b[38;5;224m";
const CYAN: &str = "\x1b[38;5;80m";
const RST: &str = "\x1b[0m";

pub fn available() -> bool {
    OpenOptions::new().read(true).write(true).open("/dev/tty").is_ok()
        && Command::new("stty")
            .arg("-g")
            .stdin(tty_in().unwrap_or(Stdio::null()))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn tty_in() -> Option<Stdio> {
    File::open("/dev/tty").ok().map(Stdio::from)
}

fn stty(args: &[&str]) -> Option<String> {
    let out = Command::new("stty").args(args).stdin(tty_in()?).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

fn restore_terminal() {
    if let Some(s) = SAVED.get() {
        stty(&[s.as_str()]);
    } else {
        stty(&["sane"]);
    }
    if let Ok(mut t) = OpenOptions::new().write(true).open("/dev/tty") {
        let _ = t.write_all(b"\x1b[0m\x1b[?25h\x1b[?1049l");
        let _ = t.flush();
    }
}

#[derive(Debug)]
enum Key {
    Char(u8),
    Up,
    Down,
    Left,
    Right,
    Esc,
}

fn bar(v: f32, w: usize) -> String {
    let v = v.clamp(0.0, 1.0);
    let full = (v * w as f32).round() as usize;
    let mut s = String::with_capacity(w * 3);
    for k in 0..w {
        s.push(if k < full { '█' } else { '░' });
    }
    s
}

pub fn run(sh: &Arc<Shared>, info: &LiveInfo) -> Result<(), String> {
    let saved = stty(&["-g"]).ok_or("cannot read terminal settings")?;
    let _ = SAVED.set(saved);
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |p| {
        restore_terminal();
        prev_hook(p);
    }));
    stty(&["-icanon", "-echo", "-isig", "min", "1", "time", "0"]).ok_or("cannot set raw mode")?;

    let mut out = OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .map_err(|e| e.to_string())?;
    let mut rd = File::open("/dev/tty").map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel::<u8>();
    std::thread::spawn(move || {
        let mut b = [0u8; 1];
        while let Ok(1) = rd.read(&mut b) {
            if tx.send(b[0]).is_err() {
                break;
            }
        }
    });

    let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J");
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut flash: Option<(String, Instant)> = None;

    'main: loop {
        let key = match rx.recv_timeout(Duration::from_millis(60)) {
            Ok(0x1b) => match rx.recv_timeout(Duration::from_millis(30)) {
                Ok(b'[') | Ok(b'O') => match rx.recv_timeout(Duration::from_millis(30)) {
                    Ok(b'A') => Some(Key::Up),
                    Ok(b'B') => Some(Key::Down),
                    Ok(b'C') => Some(Key::Right),
                    Ok(b'D') => Some(Key::Left),
                    _ => None,
                },
                Ok(_) => None,
                Err(_) => Some(Key::Esc),
            },
            Ok(b) => Some(Key::Char(b)),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break 'main,
        };

        if let Some(k) = key {
            let msg: Option<&str> = match k {
                Key::Esc | Key::Char(3) | Key::Char(b'Q') => break 'main,
                Key::Up | Key::Char(b'+') => {
                    sh.nudge("intensity", 0.05);
                    None
                }
                Key::Down => {
                    sh.nudge("intensity", -0.05);
                    None
                }
                Key::Right => {
                    sh.nudge("darkness", 0.05);
                    None
                }
                Key::Left => {
                    sh.nudge("darkness", -0.05);
                    None
                }
                Key::Char(c @ b'1'..=b'9') => {
                    sh.set_param("intensity", (c - b'1') as f32 / 8.0);
                    None
                }
                Key::Char(b',') => {
                    sh.nudge("bpm", -2.0);
                    None
                }
                Key::Char(b'.') => {
                    sh.nudge("bpm", 2.0);
                    None
                }
                Key::Char(b'[') => {
                    sh.nudge("space", -0.05);
                    None
                }
                Key::Char(b']') => {
                    sh.nudge("space", 0.05);
                    None
                }
                Key::Char(b'o') => {
                    sh.nudge("tape", -0.05);
                    None
                }
                Key::Char(b'p') => {
                    sh.nudge("tape", 0.05);
                    None
                }
                Key::Char(b'-') => {
                    sh.nudge("volume", -0.05);
                    None
                }
                Key::Char(b'=') => {
                    sh.nudge("volume", 0.05);
                    None
                }
                Key::Char(c) if b"qwertyu".contains(&c) => {
                    let idx = b"qwertyu".iter().position(|&x| x == c).unwrap();
                    sh.cycle_layer(idx);
                    None
                }
                Key::Char(b' ') => {
                    sh.fire("boom");
                    Some("BOOM")
                }
                Key::Char(b'f') => {
                    sh.fire("fill");
                    Some("fill on next bar")
                }
                Key::Char(b'g') => {
                    sh.fire("riser");
                    Some("riser building...")
                }
                Key::Char(b'n') => {
                    sh.fire("next");
                    Some("next section on next bar")
                }
                Key::Char(b'k') => {
                    sh.fire("key");
                    Some("modulating on next bar")
                }
                Key::Char(b'h') => {
                    sh.toggle("hold");
                    None
                }
                Key::Char(b'z') => {
                    sh.toggle("drift");
                    None
                }
                Key::Char(b'x') => {
                    sh.toggle("fade");
                    None
                }
                _ => None,
            };
            if let Some(m) = msg {
                flash = Some((m.to_string(), Instant::now()));
            }
        }

        if last.elapsed() >= Duration::from_millis(90) {
            last = Instant::now();
            if let Some((_, t)) = &flash {
                if t.elapsed() > Duration::from_secs(2) {
                    flash = None;
                }
            }
            let screen = draw(sh, info, flash.as_ref().map(|f| f.0.as_str()));
            if out.write_all(screen.as_bytes()).is_err() {
                break;
            }
            let _ = out.flush();
        }
    }

    restore_terminal();
    Ok(())
}

fn onoff(b: bool) -> String {
    if b {
        format!("{HOT}ON {RST}")
    } else {
        format!("{DIM}off{RST}")
    }
}

fn draw(sh: &Shared, info: &LiveInfo, flash: Option<&str>) -> String {
    let mut s = String::from("\x1b[H");
    let mut line = |t: String| {
        s.push_str("\x1b[2K");
        s.push_str(&t);
        s.push_str("\r\n");
    };
    let it = sh.intensity.get();
    let now = sh.st_intensity.get();
    let fader = sh.st_master.get();
    line(String::new());
    line(format!(
        "  {HOT}K A A M O S{RST}  {DIM}— procedural night synth —{RST}   {}",
        if fader < 0.999 && sh.fade.load(Relaxed) {
            format!("{RED}fading out{RST}")
        } else {
            format!("{RED}● live{RST}")
        }
    ));
    line(String::new());
    line(format!(
        "  {TXT}INTENSITY{RST} {RED}{}{RST} {TXT}{:.2}{RST} {DIM}now {:.2}{RST}   {TXT}DRIFT{RST} {}",
        bar(it, 30),
        it,
        now,
        onoff(sh.drift.load(Relaxed))
    ));
    line(format!(
        "  {TXT}DARKNESS {RST} {RED}{}{RST} {TXT}{:.2}{RST}          {TXT}HOLD {RST} {}",
        bar(sh.darkness.get(), 30),
        sh.darkness.get(),
        onoff(sh.hold.load(Relaxed))
    ));
    line(format!(
        "  {TXT}TEMPO{RST} {:<4.0} {TXT}SPACE{RST} {:.2}  {TXT}TAPE{RST} {:.2}  {TXT}VOLUME{RST} {:.2}   {TXT}FADE {RST} {}",
        sh.bpm.get(),
        sh.space.get(),
        sh.tape.get(),
        sh.volume.get(),
        onoff(sh.fade.load(Relaxed))
    ));
    line(String::new());
    line(format!(
        "  {TXT}KEY{RST} {CYAN}{:<20}{RST} {TXT}CHORD{RST} {CYAN}{:<9}{RST} {TXT}BAR{RST} {:<5} {TXT}SECTION{RST} {:<4} {TXT}OUT{RST} {RED}{}{RST}",
        sh.key_name(),
        sh.chord(),
        sh.st_bar.load(Relaxed),
        sh.st_section.load(Relaxed),
        bar(sh.st_peak.get(), 8)
    ));
    line(String::new());
    let keys = ['q', 'w', 'e', 'r', 't', 'y', 'u'];
    for (i, name) in LAYERS.iter().enumerate() {
        let m = sh.layer_mode[i].load(Relaxed) as usize % 3;
        let mc = match m {
            1 => HOT,
            2 => DIM,
            _ => TXT,
        };
        line(format!(
            "  {DIM}{}{RST} {TXT}{:<6}{RST} {mc}{:<4}{RST} {RED}{}{RST}",
            keys[i],
            name,
            MODE_NAMES[m],
            bar(sh.st_levels[i].get(), 16)
        ));
    }
    line(String::new());
    line(format!("  {HOT}{}{RST}", flash.unwrap_or("")));
    line(format!(
        "  {DIM}↑↓ intensity · 1-9 jump · ←→ darkness · , . tempo · [ ] space · o p tape · - = volume{RST}"
    ));
    line(format!(
        "  {DIM}q w e r t y u  layer auto/on/off · h hold · z drift · x fade out/in{RST}"
    ));
    line(format!(
        "  {DIM}SPACE boom · g riser · f fill · n next section · k new key · Esc quit{RST}"
    ));
    line(String::new());
    match &info.url {
        Some(u) => line(format!("  {TXT}phone panel{RST} {CYAN}{u}{RST}")),
        None => line(format!("  {DIM}web panel off{RST}")),
    }
    line(format!(
        "  {DIM}audio: {} @ {} Hz · seed {}{RST}",
        info.device, info.rate, info.seed
    ));
    s.push_str("\x1b[J");
    s
}
