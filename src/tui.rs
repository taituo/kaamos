//! Terminal control surface. Talks to /dev/tty directly (so stdout stays free
//! for --stdout audio) and uses `stty` for raw keys, no extra crates.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::music;
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
                Key::Char(c) if b"qwertyui".contains(&c) => {
                    let idx = b"qwertyui".iter().position(|&x| x == c).unwrap();
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
                Key::Char(b'l') => {
                    sh.fire("loop");
                    Some("pattern repeats +1")
                }
                Key::Char(b'c') => {
                    sh.fire("export");
                    Some("capturing pattern for Strudel...")
                }
                Key::Char(b'L') => {
                    sh.toggle("lock");
                    Some(if sh.lock.load(Relaxed) {
                        "pattern LOCKED"
                    } else {
                        "pattern released"
                    })
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
        "  {HOT}K A A M O S{RST}  {DIM}· procedural night synth ·{RST}   {}",
        if fader < 0.999 && sh.fade.load(Relaxed) {
            format!("{RED}fading out{RST}")
        } else {
            format!("{RED}● live{RST}")
        }
    ));
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
    let loops = sh.st_loops.load(Relaxed);
    let left = sh.st_bars_left.load(Relaxed);
    let status = if sh.hold.load(Relaxed) {
        format!("{HOT}chord held{RST}")
    } else if sh.lock.load(Relaxed) {
        format!("{HOT}LOCKED{RST} {TXT}repeating until released{RST}")
    } else if loops > 0 {
        format!("{HOT}+{loops} repeats{RST} {DIM}· changes in {left} bars{RST}")
    } else {
        format!("{DIM}changes in {left} bars{RST}")
    };
    line(format!(
        "  {TXT}PATTERN{RST} {CYAN}{} bars{RST}   {status}",
        sh.st_loop_bars.load(Relaxed)
    ));
    let export = sh.export_msg.lock().map(|s| s.clone()).unwrap_or_default();
    if export.is_empty() {
        line(format!(
            "  {DIM}EXPORT  press c to capture this pattern as Strudel code{RST}"
        ));
    } else {
        line(format!("  {TXT}EXPORT{RST}  {CYAN}{export}{RST}"));
    }
    line(String::new());
    for row in piano(sh) {
        line(row);
    }
    line(String::new());
    let keys = ['q', 'w', 'e', 'r', 't', 'y', 'u', 'i'];
    let half = LAYERS.len().div_ceil(2);
    for r in 0..half {
        let mut row = String::from("  ");
        for i in [r, r + half] {
            if i >= LAYERS.len() {
                continue;
            }
            let m = sh.layer_mode[i].load(Relaxed) as usize % 3;
            let mc = match m {
                1 => HOT,
                2 => DIM,
                _ => TXT,
            };
            row.push_str(&format!(
                "{DIM}{}{RST} {TXT}{:<6}{RST} {mc}{:<4}{RST} {RED}{}{RST}      ",
                keys[i],
                LAYERS[i],
                MODE_NAMES[m],
                bar(sh.st_levels[i].get(), 14)
            ));
        }
        line(row);
    }
    line(format!("  {HOT}{}{RST}", flash.unwrap_or("")));
    line(format!(
        "  {DIM}↑↓ intensity · 1-9 jump · ←→ darkness · , . tempo · [ ] space · o p tape · - = volume{RST}"
    ));
    line(format!(
        "  {DIM}q w e r t y u i layers · l repeat +1 · L lock pattern · h hold chord · c capture{RST}"
    ));
    line(format!(
        "  {DIM}SPACE boom · g riser · f fill · n next · k new key · z drift · x fade · Esc quit{RST}"
    ));
    let panel = match &info.url {
        Some(u) => format!("{TXT}phone{RST} {CYAN}{u}{RST}"),
        None => format!("{DIM}web panel off{RST}"),
    };
    line(format!(
        "  {panel}   {DIM}{} @ {} Hz · seed {}{RST}",
        info.device, info.rate, info.seed
    ));
    s.push_str("\x1b[J");
    s
}

// ------------------------------------------------------------------ keyboard

const LOW: i32 = 36; // C2
const HIGH: i32 = 95; // B6

const C_CHORD: u8 = 203;
const C_KEYS: u8 = 141;
const C_ARP: u8 = 209;
const C_BASS: u8 = 44;
const C_LEAD: u8 = 228;

fn is_black(n: i32) -> bool {
    matches!(n.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

fn cell(bg: u8) -> String {
    format!("\x1b[48;5;{bg}m \x1b[0m")
}

/// A white-key cell with a thin dark line on its right edge (the gap between keys).
fn edge(bg: u8) -> String {
    format!("\x1b[48;5;{bg};38;5;236m▕\x1b[0m")
}

/// A five-octave keyboard: sounding notes light up, in-key keys are brighter than
/// out-of-key ones, and ▲ marks show the likely next melody notes.
fn piano(sh: &Shared) -> Vec<String> {
    let (chord, keys, bass, arp, lead) = sh.sounding();
    let key = sh.key_root();
    let mode = sh.mode();
    let (pads, probs) = sh.suggestions();
    let color = |n: i32| -> u8 {
        if n == lead {
            C_LEAD
        } else if n == bass {
            C_BASS
        } else if n == arp {
            C_ARP
        } else if keys.contains(&n) {
            C_KEYS
        } else if chord.contains(&n) {
            C_CHORD
        } else {
            let inkey = music::in_scale(key, mode, n);
            match (is_black(n), inkey) {
                (false, true) => 253,
                (false, false) => 244,
                (true, true) => 239,
                (true, false) => 234,
            }
        }
    };
    let prob = |n: i32| pads.iter().position(|&p| p == n).map(|i| probs[i]).unwrap_or(0.0);
    let mark = |p: f32| -> String {
        if p > 0.15 {
            format!("\x1b[1;38;5;220m▲{RST}")
        } else if p > 0.08 {
            format!("\x1b[38;5;178m▲{RST}")
        } else if p > 0.04 {
            format!("\x1b[38;5;94m·{RST}")
        } else {
            " ".into()
        }
    };
    let whites: Vec<i32> = (LOW..=HIGH).filter(|&n| !is_black(n)).collect();
    let (mut top, mut bottom, mut hint, mut labels) = (
        String::from("  "),
        String::from("  "),
        String::from("  "),
        String::from("  "),
    );
    for &w in &whites {
        let b = w + 1;
        let has_black = b <= HIGH && is_black(b);
        top.push_str(&cell(color(w)));
        top.push_str(&if has_black { cell(color(b)) } else { edge(color(w)) });
        bottom.push_str(&cell(color(w)));
        bottom.push_str(&edge(color(w)));
        hint.push_str(&mark(prob(w)));
        hint.push_str(&if has_black { mark(prob(b)) } else { " ".into() });
        if w.rem_euclid(12) == 0 {
            labels.push_str(&format!("{DIM}C{}{RST}", w / 12 - 1));
        } else {
            labels.push_str("  ");
        }
    }
    let legend = format!(
        "  \x1b[38;5;{C_CHORD}m■{RST}{DIM} chord {RST}\x1b[38;5;{C_KEYS}m■{RST}{DIM} keys {RST}\x1b[38;5;{C_ARP}m■{RST}{DIM} arp {RST}\x1b[38;5;{C_BASS}m■{RST}{DIM} bass {RST}\x1b[38;5;{C_LEAD}m■{RST}{DIM} lead   {RST}\x1b[1;38;5;220m▲{RST}{DIM} likely next note   bright keys = in key{RST}"
    );
    vec![top.clone(), top, bottom, hint, labels, legend]
}
