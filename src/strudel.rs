//! Export captured patterns as Strudel code (https://strudel.cc).
//!
//! Each capture becomes `const pN = stack(...)`. Captures made with the same seed go
//! into the same file, and the closing `arrange(...)` plays them in order as a song.

use std::fs;
use std::path::{Path, PathBuf};

use crate::params::{Shared, LAYERS};

/// Everything needed to write one pattern (one pass of the chord loop) as code.
/// Built by the engine at capture time; all notes are MIDI numbers.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub seed: u64,
    pub section: u32,
    pub key_name: String,
    pub chord_names: Vec<String>,
    pub bpm: f32,
    pub intensity: f32,
    pub space: f32,
    pub loop_bars: u32,
    /// pad chord per bar
    pub pad: Vec<Vec<i32>>,
    /// arp notes over the whole loop, evenly spaced
    pub arp: Vec<i32>,
    /// (start step, length in steps, midi) over the loop
    pub bass: Vec<(u32, u32, i32)>,
    pub lead: Vec<(u32, u32, i32)>,
    pub drone: i32,
    /// sparkle pool per bar (high chord tones)
    pub bells: Vec<Vec<i32>>,
    pub drum_level: u8,
    pub drum_var: u8,
    pub gains: [f32; 7],
    pub pad_cut: f32,
    pub arp_cut: f32,
    pub bass_cut: f32,
    pub lead_cut: f32,
}

const NAMES: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];

fn nn(m: i32) -> String {
    format!("{}{}", NAMES[m.rem_euclid(12) as usize], m.div_euclid(12) - 1)
}

fn tok(t: &str, w: u32) -> String {
    if w == 1 {
        t.to_string()
    } else {
        format!("{t}@{w}")
    }
}

/// Timed notes -> mini-notation with rests and @-weights, covering `total` steps.
fn timeline(events: &[(u32, u32, String)], total: u32) -> String {
    let mut ev: Vec<&(u32, u32, String)> = events.iter().filter(|e| e.0 < total).collect();
    ev.sort_by_key(|e| e.0);
    let mut out = Vec::new();
    let mut pos = 0;
    for (k, e) in ev.iter().enumerate() {
        if e.0 > pos {
            out.push(tok("~", e.0 - pos));
        }
        if e.0 < pos {
            continue;
        }
        let next = ev.get(k + 1).map(|n| n.0).unwrap_or(total).min(total);
        let len = e.1.clamp(1, (next - e.0).max(1));
        out.push(tok(&e.2, len));
        pos = e.0 + len;
    }
    if pos < total {
        out.push(tok("~", total - pos));
    }
    out.join(" ")
}

/// One bracketed group per bar; identical neighbouring bars collapse to `[..]!n`.
fn per_bar(chunks: &[String]) -> String {
    let mut out: Vec<(String, u32)> = Vec::new();
    for c in chunks {
        match out.last_mut() {
            Some((last, n)) if last == c => *n += 1,
            _ => out.push((c.clone(), 1)),
        }
    }
    out.iter()
        .map(|(c, n)| {
            if *n == 1 {
                format!("[{c}]")
            } else {
                format!("[{c}]!{n}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn grid(hits: &[u32], name: &str) -> String {
    (0..16)
        .map(|s| if hits.contains(&s) { name } else { "~" })
        .collect::<Vec<_>>()
        .join(" ")
}

fn g(x: f32) -> String {
    format!("{:.2}", x.max(0.0))
}

/// Code for one pattern, named `name`, as a self-contained `const`.
pub fn pattern_code(s: &Snapshot, name: &str) -> String {
    let bars = s.loop_bars.max(1);
    let steps = bars * 16;
    let echo = 0.75 * 60.0 / s.bpm;
    let room = 0.35 + 0.5 * s.space;
    let mut layers: Vec<(usize, String)> = Vec::new();

    // drone: one long note over the loop
    layers.push((
        0,
        format!(
            "note(\"{}\").s(\"sawtooth\").lpf(160).lpq(6).attack(2).release(4).gain({}).slow({bars})",
            nn(s.drone),
            g(0.45 * s.gains[0])
        ),
    ));

    // pad: one chord per bar
    let pad: Vec<(u32, u32, String)> = s
        .pad
        .iter()
        .enumerate()
        .map(|(b, ch)| {
            (
                b as u32 * 16,
                16,
                format!("[{}]", ch.iter().map(|&n| nn(n)).collect::<Vec<_>>().join(",")),
            )
        })
        .collect();
    layers.push((
        1,
        format!(
            "note(\"{}\").slow({bars})\n    .s(\"sawtooth\").lpf({:.0}).lpq(4).attack(1.2).decay(1).sustain(.85).release(2.5)\n    .room({:.2}).roomsize(6).gain({})",
            merge_repeats(&timeline(&pad, steps)),
            s.pad_cut,
            room,
            g(0.32 * s.gains[1])
        ),
    ));

    // arp
    if !s.arp.is_empty() {
        let per = (s.arp.len() / bars as usize).max(1);
        let chunks: Vec<String> = s
            .arp
            .chunks(per)
            .map(|c| c.iter().map(|&n| nn(n)).collect::<Vec<_>>().join(" "))
            .collect();
        let arp = per_bar(&chunks);
        layers.push((
            2,
            format!(
                "note(\"{arp}\").slow({bars})\n    .s(\"sawtooth\").lpf({:.0}).lpq(12).lpenv(3).lpdecay(.15).decay(.25).sustain(.3)\n    .delay(.35).delaytime({:.3}).delayfeedback(.45).room({:.2}).gain({})",
                s.arp_cut,
                echo,
                room * 0.6,
                g(0.4 * s.gains[2])
            ),
        ));
    }

    // bass
    let bass_bars: Vec<String> = (0..bars)
        .map(|b| {
            let ev: Vec<(u32, u32, String)> = s
                .bass
                .iter()
                .filter(|e| e.0 / 16 == b)
                .map(|&(st, len, n)| (st - b * 16, len, nn(n)))
                .collect();
            timeline(&ev, 16)
        })
        .collect();
    layers.push((
        3,
        format!(
            "note(\"{}\").slow({bars})\n    .s(\"sawtooth\").lpf({:.0}).lpq(6).lpenv(2).lpdecay(.1).decay(.2).sustain(.55).clip(.85).gain({})",
            per_bar(&bass_bars),
            s.bass_cut,
            g(0.55 * s.gains[3])
        ),
    ));

    // drums
    layers.push((4, drums(s)));

    // lead
    if !s.lead.is_empty() {
        let lead: Vec<(u32, u32, String)> = s.lead.iter().map(|&(st, len, n)| (st, len, nn(n))).collect();
        layers.push((
            5,
            format!(
                "note(\"{}\").slow({bars})\n    .s(\"square\").lpf({:.0}).attack(.03).decay(.5).sustain(.7).release(.4)\n    .delay(.45).delaytime({:.3}).delayfeedback(.45).room({:.2}).gain({})",
                timeline(&lead, steps),
                s.lead_cut,
                echo,
                room,
                g(0.3 * s.gains[5])
            ),
        ));
    }

    // bells: random sparkles drawn from the chord tones
    let bell_bars: Vec<String> = s
        .bells
        .iter()
        .map(|pool| {
            let t: Vec<String> = pool.iter().map(|&n| nn(n)).collect();
            format!(
                "{} {}",
                t.join(" "),
                t.iter().rev().cloned().collect::<Vec<_>>().join(" ")
            )
        })
        .collect();
    let bells = per_bar(&bell_bars);
    layers.push((
        6,
        format!(
            "note(\"{bells}\").slow({bars}).degradeBy(.8)\n    .s(\"sine\").fm(2.5).fmh(3.5).decay(1.2).sustain(0).pan(rand).room(.8).gain({})",
            g(0.3 * s.gains[6])
        ),
    ));

    let mut out = format!(
        "// {name} · section {} · {} · {} · {bars} bars · intensity {:.2} · {:.0} bpm\nconst {name} = stack(\n",
        s.section,
        s.key_name,
        s.chord_names.join(" "),
        s.intensity,
        s.bpm
    );
    for (li, code) in layers {
        let on = s.gains[li] > 0.05;
        let label = LAYERS[li];
        if on {
            out.push_str(&format!("  // {label}\n  {code},\n"));
        } else {
            out.push_str(&format!("  // {label} (silent at this intensity)\n"));
            for l in format!("{code},").lines() {
                out.push_str(&format!("  // {}\n", l.trim_start()));
            }
        }
    }
    out.push_str(")\n");
    out
}

/// "[a]@16 [a]@16 [b]@16" -> "[a]@32 [b]@16" (consecutive identical chords merged)
fn merge_repeats(seq: &str) -> String {
    let mut out: Vec<(String, u32)> = Vec::new();
    for t in seq.split(' ') {
        let (name, w) = match t.rsplit_once('@') {
            Some((n, w)) => (n.to_string(), w.parse::<u32>().unwrap_or(1)),
            None => (t.to_string(), 1),
        };
        match out.last_mut() {
            Some((last, lw)) if *last == name => *lw += w,
            _ => out.push((name, w)),
        }
    }
    out.iter().map(|(n, w)| tok(n, *w)).collect::<Vec<_>>().join(" ")
}

fn drums(s: &Snapshot) -> String {
    let v = s.drum_var;
    let bank = ".bank(\"RolandTR808\")";
    let gain = 0.9 * s.gains[4];
    let parts: Vec<String> = match s.drum_level {
        0 => vec![format!("s(\"{}\"){bank}.lpf(200).gain(.9)", grid(&[0, 3], "bd"))],
        1 => {
            let mut k = vec![0, 8];
            if v == 1 {
                k.push(10);
            }
            vec![
                format!("s(\"{}\"){bank}", grid(&k, "bd")),
                format!("s(\"{}\"){bank}.gain(.35)", grid(&[2, 6, 10, 14], "hh")),
            ]
        }
        2 => {
            let mut k = vec![0, 8];
            if v == 1 {
                k.push(6);
            }
            if v == 2 {
                k.push(10);
            }
            let hats: Vec<u32> = if v == 2 {
                (0..16).collect()
            } else {
                (0..16).step_by(2).collect()
            };
            vec![
                format!("s(\"{}\"){bank}", grid(&k, "bd")),
                format!("s(\"{}\"){bank}.room(.4).roomsize(.9)", grid(&[4, 12], "sd")),
                format!("s(\"{}\"){bank}.gain(.3)", grid(&hats, "hh")),
            ]
        }
        _ => vec![
            format!("s(\"{}\"){bank}", grid(&[0, 4, 8, 12], "bd")),
            format!("s(\"{}\"){bank}.room(.4).roomsize(.9)", grid(&[4, 12], "sd")),
            format!(
                "s(\"{}\"){bank}.gain(.3)",
                grid(&[0, 1, 3, 4, 5, 7, 8, 9, 11, 12, 13, 15], "hh")
            ),
            format!("s(\"{}\"){bank}.gain(.35)", grid(&[2, 6, 10, 14], "oh")),
        ],
    };
    format!("stack(\n    {}\n  ).gain({})", parts.join(",\n    "), g(gain))
}

// ------------------------------------------------------------------ file handling

const SONG: &str = "// @song";

pub struct Exporter {
    dir: PathBuf,
    last_body: Option<(u64, String)>,
}

impl Exporter {
    pub fn new(dir: &str) -> Self {
        Exporter {
            dir: PathBuf::from(dir),
            last_body: None,
        }
    }

    pub fn path_for(&self, seed: u64) -> PathBuf {
        self.dir.join(format!("kaamos-{seed}.strudel.js"))
    }

    /// Write any pending captures. Returns a short status message if something happened.
    pub fn flush(&mut self, sh: &Shared) -> Option<String> {
        let caps: Vec<Snapshot> = match sh.captures.lock() {
            Ok(mut v) => v.drain(..).collect(),
            Err(_) => return None,
        };
        let mut msg = None;
        for cap in caps {
            let m = match self.append(&cap) {
                Ok(m) => m,
                Err(e) => format!("export failed: {e}"),
            };
            if let Ok(mut s) = sh.export_msg.lock() {
                *s = m.clone();
            }
            msg = Some(m);
        }
        msg
    }

    fn append(&mut self, cap: &Snapshot) -> std::io::Result<String> {
        let body = pattern_code(cap, "PAT");
        if self.last_body.as_ref() == Some(&(cap.seed, body.clone())) {
            return Ok("same pattern already captured".into());
        }
        let path = self.path_for(cap.seed);
        let existing = fs::read_to_string(&path).ok();
        let (head, mut song) = match &existing {
            Some(text) => split_song(text),
            None => (header(cap), Vec::new()),
        };
        let name = format!("p{}", song.len() + 1);
        let bars = (16 / cap.loop_bars.max(1)).max(1) * cap.loop_bars.max(1);
        song.push((name.clone(), bars));
        let code = pattern_code(cap, &name);
        let mut out = head.trim_end().to_string();
        out.push_str("\n\n");
        out.push_str(&code);
        out.push('\n');
        out.push_str(&song_lines(&song));
        write_file(&path, &out)?;
        self.last_body = Some((cap.seed, body));
        Ok(format!("captured {name} -> {}", display(&path)))
    }
}

fn display(p: &Path) -> String {
    p.strip_prefix(".").unwrap_or(p).display().to_string()
}

fn write_file(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir)?;
        }
    }
    fs::write(path, text)
}

fn header(cap: &Snapshot) -> String {
    format!(
        "// KAAMOS patterns · seed {}\n\
         // Paste into https://strudel.cc and press Ctrl+Enter (Cmd+Enter on a Mac).\n\
         // Each capture in kaamos (key c) with this seed is appended below.\n\
         // The arrange() at the end plays them in order; edit the bar counts to taste.\n\
         setcps({:.0}/60/4)\n",
        cap.seed, cap.bpm
    )
}

/// Split a file into (everything before the song marker, [(name, bars)]).
fn split_song(text: &str) -> (String, Vec<(String, u32)>) {
    match text.find(SONG) {
        Some(i) => {
            let line = text[i..].lines().next().unwrap_or("");
            let song = line[SONG.len()..]
                .split_whitespace()
                .filter_map(|t| {
                    let (n, b) = t.split_once(':')?;
                    Some((n.to_string(), b.parse().ok()?))
                })
                .collect();
            (text[..i].to_string(), song)
        }
        None => (text.to_string(), Vec::new()),
    }
}

fn song_lines(song: &[(String, u32)]) -> String {
    let marker: Vec<String> = song.iter().map(|(n, b)| format!("{n}:{b}")).collect();
    let parts: Vec<String> = song.iter().map(|(n, b)| format!("[{b}, {n}]")).collect();
    format!("{SONG} {}\narrange({})\n", marker.join(" "), parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(seed: u64) -> Snapshot {
        Snapshot {
            seed,
            section: 2,
            key_name: "A aeolian".into(),
            chord_names: vec!["Am7".into(), "Fmaj7".into()],
            bpm: 92.0,
            intensity: 0.7,
            space: 0.6,
            loop_bars: 4,
            pad: vec![
                vec![57, 60, 64, 67],
                vec![57, 60, 64, 67],
                vec![53, 57, 60, 64],
                vec![53, 57, 60, 64],
            ],
            arp: vec![57, 60, 64, 67, 64, 60, 57, 60],
            bass: vec![(0, 2, 33), (8, 2, 33), (32, 2, 29), (40, 2, 29)],
            lead: vec![(0, 8, 69), (12, 20, 72), (40, 8, 71)],
            drone: 33,
            bells: vec![vec![76, 79, 83, 86]; 4],
            drum_level: 2,
            drum_var: 1,
            gains: [0.8, 1.0, 1.0, 1.0, 1.0, 0.0, 0.5],
            pad_cut: 1800.0,
            arp_cut: 1200.0,
            bass_cut: 400.0,
            lead_cut: 2000.0,
        }
    }

    #[test]
    fn timeline_and_names() {
        assert_eq!(nn(60), "c4");
        assert_eq!(nn(33), "a1");
        let ev = vec![(2, 4, "a".to_string()), (8, 20, "b".to_string())];
        assert_eq!(timeline(&ev, 16), "~@2 a@4 ~@2 b@8");
        assert_eq!(merge_repeats("[x]@16 [x]@16 [y]@16"), "[x]@32 [y]@16");
        let c = vec!["a b".to_string(), "a b".to_string(), "c".to_string()];
        assert_eq!(per_bar(&c), "[a b]!2 [c]");
    }

    #[test]
    fn appends_with_same_seed_and_rebuilds_arrange() {
        let dir = std::env::temp_dir().join(format!("kaamos-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut ex = Exporter::new(dir.to_str().unwrap());
        let mut a = snap(5);
        ex.append(&a).unwrap();
        a.section = 3;
        a.drum_level = 3;
        ex.append(&a).unwrap();
        // identical capture is skipped
        assert!(ex.append(&a).unwrap().contains("already"));
        let text = fs::read_to_string(ex.path_for(5)).unwrap();
        assert_eq!(text.matches("const p").count(), 2);
        assert!(text.contains("arrange([16, p1], [16, p2])"));
        assert_eq!(text.matches("setcps").count(), 1);
        // a different seed starts a new file
        ex.append(&snap(6)).unwrap();
        assert!(ex.path_for(6).exists());
        // balanced brackets and quotes, so the JS at least parses structurally
        assert_eq!(text.matches('(').count(), text.matches(')').count());
        assert_eq!(text.matches('[').count(), text.matches(']').count());
        assert_eq!(text.matches('"').count() % 2, 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
