//! Harmony and pattern generation: modes, progressions, voicings, motifs, melodies.

use crate::dsp::Rng;

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub struct Mode {
    pub name: &'static str,
    pub iv: [i32; 7],
}

/// Ordered bright -> dark. Darkness 0..1 maps onto this list.
pub const MODES: [Mode; 7] = [
    Mode {
        name: "lydian",
        iv: [0, 2, 4, 6, 7, 9, 11],
    },
    Mode {
        name: "ionian",
        iv: [0, 2, 4, 5, 7, 9, 11],
    },
    Mode {
        name: "mixolydian",
        iv: [0, 2, 4, 5, 7, 9, 10],
    },
    Mode {
        name: "dorian",
        iv: [0, 2, 3, 5, 7, 9, 10],
    },
    Mode {
        name: "aeolian",
        iv: [0, 2, 3, 5, 7, 8, 10],
    },
    Mode {
        name: "harmonic minor",
        iv: [0, 2, 3, 5, 7, 8, 11],
    },
    Mode {
        name: "phrygian",
        iv: [0, 1, 3, 5, 7, 8, 10],
    },
];

/// MIDI note of scale degree `d` (0 = tonic, can be negative / > 6) above `root`.
#[inline]
pub fn deg(root: i32, mode: usize, d: i32) -> i32 {
    root + 12 * d.div_euclid(7) + MODES[mode].iv[d.rem_euclid(7) as usize]
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Quality {
    Maj,
    Min,
    Dim,
    Aug,
}

pub fn triad_quality(mode: usize, d: i32) -> Quality {
    let r = deg(0, mode, d);
    let t = deg(0, mode, d + 2) - r;
    let f = deg(0, mode, d + 4) - r;
    match (t, f) {
        (4, 7) => Quality::Maj,
        (3, 7) => Quality::Min,
        (3, 6) => Quality::Dim,
        _ => Quality::Aug,
    }
}

pub fn chord_name(root: i32, mode: usize, d: i32, ext: usize) -> String {
    let pc = deg(root, mode, d).rem_euclid(12) as usize;
    let q = triad_quality(mode, d);
    let r = deg(0, mode, d);
    let sev = deg(0, mode, d + 6) - r;
    let base = match q {
        Quality::Maj => "",
        Quality::Min => "m",
        Quality::Dim => "dim",
        Quality::Aug => "+",
    };
    let ext_s = if ext >= 4 {
        match (q, sev) {
            (Quality::Maj, 11) => {
                if ext >= 5 {
                    "maj9"
                } else {
                    "maj7"
                }
            }
            (Quality::Maj, 10) => {
                if ext >= 5 {
                    "9"
                } else {
                    "7"
                }
            }
            (Quality::Min, 10) => {
                if ext >= 5 {
                    "9"
                } else {
                    "7"
                }
            }
            _ => "",
        }
    } else {
        ""
    };
    format!("{}{}{}", NOTE_NAMES[pc], base, ext_s)
}

/// Progression templates as scale degrees (0-based). Filtered per mode so that
/// only major/minor triads are used.
const PROGS: &[&[i32]] = &[
    &[0, 5, 2, 6],
    &[0, 5, 6, 4],
    &[0, 6, 5, 6],
    &[0, 2, 6, 5],
    &[5, 6, 0, 0],
    &[0, 3, 5, 4],
    &[0, 5, 3, 6],
    &[0, 3, 0, 5],
    &[0, 4, 5, 3],
    &[0, 3, 0, 4],
    &[5, 3, 0, 4],
    &[0, 1, 0, 1],
    &[0, 1, 5, 4],
    &[0, 3, 6, 3],
    &[0, 6, 3, 0],
    &[0, 2, 3, 6],
    &[0, 5, 3, 4],
    &[0, 1, 0, 6],
    &[0, 6, 2, 3],
    &[0, 4, 0, 4],
    &[0, 3, 4, 0],
    &[0, 5],
    &[0, 6],
    &[0, 3],
    &[0, 1],
    &[0, 2],
    &[0, 4],
    &[0, 0, 5, 5],
    &[0, 0, 3, 3],
];

pub fn valid_progs(mode: usize) -> Vec<&'static [i32]> {
    PROGS
        .iter()
        .copied()
        .filter(|p| {
            p.iter()
                .all(|&d| matches!(triad_quality(mode, d), Quality::Maj | Quality::Min))
        })
        .collect()
}

pub fn pick_prog(rng: &mut Rng, mode: usize, intensity: f32) -> Vec<i32> {
    let v = valid_progs(mode);
    let w: Vec<f32> = v
        .iter()
        .map(|p| {
            let distinct = {
                let mut d: Vec<i32> = p.to_vec();
                d.sort();
                d.dedup();
                d.len()
            };
            if distinct <= 2 {
                1.4 - intensity
            } else {
                0.35 + intensity
            }
        })
        .collect();
    v[rng.weighted(&w)].to_vec()
}

/// Darkness 0..1 -> mode index, with some randomness around the centre.
pub fn pick_mode(rng: &mut Rng, darkness: f32) -> usize {
    let c = darkness.clamp(0.0, 1.0) * 6.0;
    let w: Vec<f32> = (0..7)
        .map(|i| (-((i as f32 - c).powi(2)) / (2.0 * 0.7 * 0.7)).exp())
        .collect();
    rng.weighted(&w)
}

/// Close voicing of chord on degree `d` with `ext` tones (3 = triad, 4 = 7th, 5 = 9th),
/// each tone placed within +-6 semitones of `center`.
pub fn voicing(root: i32, mode: usize, d: i32, ext: usize, center: f32) -> Vec<i32> {
    let mut v: Vec<i32> = (0..ext as i32)
        .map(|j| {
            let mut n = deg(root, mode, d + 2 * j);
            while (n as f32) < center - 6.0 {
                n += 12;
            }
            while (n as f32) > center + 6.0 {
                n -= 12;
            }
            n
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Arpeggio tones: ascending chord tones starting from a root placed in [lo, lo+11].
pub fn arp_tones(root: i32, mode: usize, d: i32, ext: usize, lo: i32) -> Vec<i32> {
    let mut r = deg(root, mode, d);
    while r < lo {
        r += 12;
    }
    while r >= lo + 12 {
        r -= 12;
    }
    let base = deg(0, mode, d);
    (0..ext as i32).map(|j| r + deg(0, mode, d + 2 * j) - base).collect()
}

/// An arpeggio motif: indices into the (octave-extended) chord tone list.
pub fn arp_motif(rng: &mut Rng, n: usize) -> Vec<usize> {
    let n = n.max(3);
    let top = n + 1; // allow reaching octave+third
    let m: Vec<usize> = match rng.below(7) {
        0 => (0..8).map(|i| i % (top + 1)).collect(),
        1 => {
            let mut up: Vec<usize> = (0..=n).collect();
            let mut down: Vec<usize> = (1..n).rev().collect();
            up.append(&mut down);
            up
        }
        2 => {
            let a = 1 + rng.below(top);
            let b = 1 + rng.below(top);
            let c = 1 + rng.below(top);
            vec![0, a, 0, b, 0, c, 0, b]
        }
        3 => vec![0, 2, 1, 3, 2, 4, 3, 1],
        4 => {
            let mut v = vec![0];
            for _ in 0..7 {
                v.push(rng.below(top + 1));
            }
            v
        }
        5 => vec![0, n, 1, n, 2, n, 1, n],
        _ => vec![0, 1, 2, n, 2, 1, 0, 2],
    };
    m
}

/// Bass patterns per 16-step bar: (step, semitone offset from root, length in steps, accent).
pub type BassPat = &'static [(u8, i8, f32, f32)];

pub const BASS_LOW: &[BassPat] = &[
    &[(0, 0, 6.0, 1.0), (6, 0, 2.0, 0.6), (8, 0, 6.0, 0.9), (14, 0, 1.5, 0.5)],
    &[
        (0, 0, 1.6, 1.0),
        (2, 0, 1.6, 0.6),
        (4, 0, 1.6, 0.8),
        (6, 0, 1.6, 0.6),
        (8, 0, 1.6, 0.9),
        (10, 0, 1.6, 0.6),
        (12, 0, 1.6, 0.8),
        (14, 0, 1.6, 0.6),
    ],
    &[(0, 0, 10.0, 1.0), (10, 0, 2.0, 0.6), (12, 7, 4.0, 0.7)],
];

pub const BASS_MID: &[BassPat] = &[
    &[
        (0, 0, 1.6, 1.0),
        (2, 0, 1.6, 0.6),
        (4, 0, 1.6, 0.8),
        (6, 0, 1.6, 0.6),
        (8, 0, 1.6, 0.9),
        (10, 0, 1.6, 0.6),
        (12, 0, 1.6, 0.8),
        (14, 0, 1.6, 0.6),
    ],
    &[
        (0, 0, 1.6, 1.0),
        (2, 12, 1.4, 0.7),
        (4, 0, 1.6, 0.8),
        (6, 12, 1.4, 0.7),
        (8, 0, 1.6, 0.9),
        (10, 12, 1.4, 0.7),
        (12, 0, 1.6, 0.8),
        (14, 12, 1.4, 0.7),
    ],
    &[
        (0, 0, 1.6, 1.0),
        (2, 0, 0.8, 0.6),
        (3, 0, 0.8, 0.6),
        (4, 0, 1.6, 0.9),
        (6, 0, 0.8, 0.6),
        (7, 0, 0.8, 0.6),
        (8, 0, 1.6, 0.9),
        (10, 0, 0.8, 0.6),
        (11, 0, 0.8, 0.6),
        (12, 0, 1.6, 0.9),
        (14, 0, 0.8, 0.6),
        (15, 0, 0.8, 0.6),
    ],
];

pub const BASS_HIGH: &[BassPat] = &[
    &[
        (0, 0, 0.8, 1.0),
        (1, 0, 0.8, 0.5),
        (2, 0, 0.8, 0.7),
        (3, 0, 0.8, 0.5),
        (4, 0, 0.8, 0.9),
        (5, 0, 0.8, 0.5),
        (6, 0, 0.8, 0.7),
        (7, 0, 0.8, 0.5),
        (8, 0, 0.8, 1.0),
        (9, 0, 0.8, 0.5),
        (10, 0, 0.8, 0.7),
        (11, 0, 0.8, 0.5),
        (12, 0, 0.8, 0.9),
        (13, 0, 0.8, 0.5),
        (14, 0, 0.8, 0.7),
        (15, 12, 0.8, 0.6),
    ],
    &[
        (0, 0, 0.8, 1.0),
        (1, 0, 0.8, 0.5),
        (2, 12, 0.8, 0.7),
        (3, 0, 0.8, 0.5),
        (4, 0, 0.8, 0.9),
        (5, 0, 0.8, 0.5),
        (6, 12, 0.8, 0.7),
        (7, 0, 0.8, 0.5),
        (8, 0, 0.8, 1.0),
        (9, 0, 0.8, 0.5),
        (10, 12, 0.8, 0.7),
        (11, 0, 0.8, 0.5),
        (12, 0, 0.8, 0.9),
        (13, 7, 0.8, 0.5),
        (14, 12, 0.8, 0.7),
        (15, 7, 0.8, 0.5),
    ],
    &[
        (0, 0, 1.6, 1.0),
        (2, 0, 0.8, 0.6),
        (3, 0, 0.8, 0.6),
        (4, 0, 1.6, 0.9),
        (6, 0, 0.8, 0.6),
        (7, 0, 0.8, 0.6),
        (8, 0, 1.6, 0.9),
        (10, 0, 0.8, 0.6),
        (11, 0, 0.8, 0.6),
        (12, 0, 1.6, 0.9),
        (14, 12, 0.8, 0.6),
        (15, 0, 0.8, 0.6),
    ],
];

#[derive(Clone, Copy, Debug)]
pub struct LeadNote {
    pub start: u32,
    pub dur: u32,
    pub midi: i32,
}

const RHYTHMS: &[&[(u32, u32)]] = &[
    &[(0, 8), (8, 4), (12, 4), (16, 12), (28, 4)],
    &[(0, 6), (6, 6), (12, 4), (16, 16)],
    &[(0, 4), (4, 4), (8, 8), (16, 6), (22, 2), (24, 8)],
    &[(0, 12), (12, 2), (14, 2), (16, 16)],
    &[(2, 6), (8, 8), (16, 4), (20, 4), (24, 8)],
    &[(0, 3), (3, 3), (6, 10), (16, 3), (19, 3), (22, 10)],
    &[(0, 16), (16, 8), (24, 8)],
];

/// Generate a lead line over one loop of the progression.
/// Degrees are relative to `root` (tonic around MIDI 45-56); output in ~MIDI 62-86.
pub fn gen_lead(rng: &mut Rng, root: i32, mode: usize, prog: &[i32], bars_per_chord: u32) -> Vec<LeadNote> {
    let loop_steps = prog.len() as u32 * bars_per_chord * 16;
    let phrases = (loop_steps / 32).max(1);
    let ra = rng.below(RHYTHMS.len());
    let mut rb = rng.below(RHYTHMS.len());
    if rb == ra {
        rb = (rb + 1) % RHYTHMS.len();
    }
    let chord_at = |step: u32| prog[((step / 16 / bars_per_chord) as usize) % prog.len()];
    let lead_root = root + 12;
    let (lo, hi) = (3, 13); // degree range
    let mut prev: i32 = 7 + rng.below(3) as i32;
    let mut out = Vec::new();
    let mut phrase_a: Vec<(i32, i32)> = Vec::new();
    for p in 0..phrases {
        let is_a = p % 2 == 0;
        let rhythm = RHYTHMS[if is_a { ra } else { rb }];
        let last_phrase = p == phrases - 1;
        for (ni, &(st, dur)) in rhythm.iter().enumerate() {
            let abs = p * 32 + st;
            if abs >= loop_steps {
                break;
            }
            let cd = chord_at(abs);
            let strong = st % 8 == 0;
            let last_note = ni == rhythm.len() - 1;
            let mut d;
            if is_a && p >= 2 && ni < phrase_a.len() && !(last_phrase && last_note) && rng.chance(0.75) {
                // repeat phrase A: literally, or as a diatonic sequence following the chord
                let (ad, acd) = phrase_a[ni];
                d = if rng.chance(0.5) { ad } else { ad + (cd - acd) };
            } else if strong || last_note {
                // nearest chord tone to prev, with a small random push
                let push = rng.below(5) as i32 - 2;
                let target = prev + push;
                let mut best = target;
                let mut best_dist = i32::MAX;
                for k in -2..=3 {
                    for j in 0..3 {
                        let cand = cd + 2 * j + 7 * k;
                        let dist = (cand - target).abs();
                        if cand >= lo && cand <= hi && dist < best_dist {
                            best = cand;
                            best_dist = dist;
                        }
                    }
                }
                d = best;
                if last_phrase && last_note {
                    // resolve to chord root
                    let mut r = cd;
                    while r < lo {
                        r += 7;
                    }
                    while r > hi {
                        r -= 7;
                    }
                    d = r;
                }
            } else {
                let step = *rng.pick(&[-2, -1, -1, 1, 1, 2]);
                d = prev + step;
            }
            while d < lo {
                d += 7;
            }
            while d > hi {
                d -= 7;
            }
            if is_a && p == 0 {
                phrase_a.push((d, cd));
            }
            prev = d;
            out.push(LeadNote {
                start: abs,
                dur,
                midi: deg(lead_root, mode, d),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_progressions() {
        for m in 0..MODES.len() {
            let v = valid_progs(m);
            assert!(v.len() >= 3, "mode {} has only {} progs", MODES[m].name, v.len());
            assert!(
                v.iter().any(|p| p.len() >= 4),
                "mode {} lacks 4-chord progs",
                MODES[m].name
            );
        }
    }

    #[test]
    fn voicing_is_close() {
        let v = voicing(57, 4, 5, 4, 64.0);
        assert!(v.iter().all(|&n| (58..=70).contains(&n)), "{v:?}");
    }

    #[test]
    fn lead_in_range() {
        let mut rng = Rng::new(3);
        for s in 0..50 {
            let mut r2 = Rng::new(s);
            let mode = pick_mode(&mut r2, 0.6);
            let prog = pick_prog(&mut rng, mode, 0.8);
            let l = gen_lead(&mut rng, 45 + (s as i32 % 12), mode, &prog, 1);
            assert!(!l.is_empty());
            for n in l {
                assert!((60..=92).contains(&n.midi), "{n:?}");
            }
        }
    }
}
