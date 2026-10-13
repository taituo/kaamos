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

/// A voicing that leaves room for the melody: the chord's 7th and 9th are dropped when
/// they would rub a semitone against a note the melody plays over this chord (a minor
/// second / minor ninth between melody and accompaniment is the harshest interval).
pub fn voicing_clear(root: i32, mode: usize, d: i32, ext: usize, center: f32, melody_pcs: &[i32]) -> Vec<i32> {
    let v = voicing(root, mode, d, ext, center);
    if melody_pcs.is_empty() || ext <= 3 {
        return v;
    }
    let ext_pcs: Vec<i32> = (3..ext as i32)
        .map(|j| deg(root, mode, d + 2 * j).rem_euclid(12))
        .collect();
    let kept: Vec<i32> = v
        .iter()
        .copied()
        .filter(|&n| {
            let pc = n.rem_euclid(12);
            !(ext_pcs.contains(&pc) && melody_pcs.iter().any(|&m| matches!((m - pc).rem_euclid(12), 1 | 11)))
        })
        .collect();
    if kept.len() >= 3 {
        kept
    } else {
        v
    }
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

// ---- the first generator, kept only for the before/after melody report in the tests
#[cfg(test)]
type Rhythm = &'static [(u32, u32)];

#[cfg(test)]
const RHYTHMS: &[Rhythm] = &[
    &[(0, 8), (8, 4), (12, 4), (16, 12), (28, 4)],
    &[(0, 6), (6, 6), (12, 4), (16, 16)],
    &[(0, 4), (4, 4), (8, 8), (16, 6), (22, 2), (24, 8)],
    &[(0, 12), (12, 2), (14, 2), (16, 16)],
    &[(2, 6), (8, 8), (16, 4), (20, 4), (24, 8)],
    &[(0, 3), (3, 3), (6, 10), (16, 3), (19, 3), (22, 10)],
    &[(0, 16), (16, 8), (24, 8)],
];

#[cfg(test)]
const VERSE_RHYTHMS: &[Rhythm] = &[
    &[(0, 4), (4, 4), (8, 8), (24, 8)],
    &[(2, 2), (4, 4), (8, 8), (18, 2), (20, 4), (24, 6)],
    &[(0, 6), (6, 2), (8, 8), (16, 6), (22, 2), (24, 4)],
    &[(0, 8), (8, 4), (12, 4), (16, 12)],
    &[(4, 4), (8, 4), (12, 4), (16, 16)],
];

#[cfg(test)]
const CHORUS_RHYTHMS: &[Rhythm] = &[
    &[(0, 3), (3, 3), (6, 2), (8, 8), (16, 3), (19, 3), (22, 2), (24, 8)],
    &[(0, 4), (4, 4), (8, 4), (12, 4), (16, 8), (24, 8)],
    &[(0, 8), (8, 8), (16, 6), (22, 2), (24, 8)],
    &[(0, 2), (2, 2), (4, 4), (8, 8), (16, 2), (18, 2), (20, 4), (24, 8)],
    &[(0, 6), (6, 6), (12, 4), (16, 16)],
];

#[cfg(test)]
pub struct LeadStyle {
    pub lo: i32,
    pub hi: i32,
    pub start: i32,
    pub rhythms: &'static [Rhythm],
    /// chance that phrases 3, 5, … repeat phrase 1
    pub repeat: f32,
    /// chance that a repeated note is literal rather than a sequence following the chord
    pub literal: f32,
    /// chance that the answering phrase reuses the first phrase's rhythm
    pub same_rhythm: f32,
}

#[cfg(test)]
pub const FREE_STYLE: LeadStyle = LeadStyle {
    lo: 3,
    hi: 13,
    start: 7,
    rhythms: RHYTHMS,
    repeat: 0.75,
    literal: 0.5,
    same_rhythm: 0.0,
};
#[cfg(test)]
pub const VERSE_STYLE: LeadStyle = LeadStyle {
    lo: 0,
    hi: 9,
    start: 4,
    rhythms: VERSE_RHYTHMS,
    repeat: 0.8,
    literal: 0.6,
    same_rhythm: 0.3,
};
#[cfg(test)]
pub const CHORUS_STYLE: LeadStyle = LeadStyle {
    lo: 4,
    hi: 13,
    start: 9,
    rhythms: CHORUS_RHYTHMS,
    repeat: 1.0,
    literal: 0.75,
    same_rhythm: 0.6,
};

/// Lead line for free mode.
pub fn gen_lead(rng: &mut Rng, root: i32, mode: usize, prog: &[i32], bars_per_chord: u32) -> Vec<LeadNote> {
    compose_melody(rng, root, mode, prog, bars_per_chord, Role::Free)
}

#[cfg(test)]
pub fn gen_lead_styled(
    rng: &mut Rng,
    root: i32,
    mode: usize,
    prog: &[i32],
    bars_per_chord: u32,
    style: &LeadStyle,
) -> Vec<LeadNote> {
    let loop_steps = prog.len() as u32 * bars_per_chord * 16;
    let phrases = (loop_steps / 32).max(1);
    let rs = style.rhythms;
    let ra = rng.below(rs.len());
    let rb = if rng.chance(style.same_rhythm) {
        ra
    } else {
        let mut b = rng.below(rs.len());
        if b == ra {
            b = (b + 1) % rs.len();
        }
        b
    };
    let chord_at = |step: u32| prog[((step / 16 / bars_per_chord) as usize) % prog.len()];
    let lead_root = root + 12;
    let (lo, hi) = (style.lo, style.hi);
    let mut prev: i32 = style.start + rng.below(3) as i32;
    let mut out = Vec::new();
    let mut phrase_a: Vec<(i32, i32)> = Vec::new();
    for p in 0..phrases {
        let is_a = p % 2 == 0;
        let rhythm = rs[if is_a { ra } else { rb }];
        let last_phrase = p == phrases - 1;
        let repeat_phrase = is_a && p >= 2 && rng.chance(style.repeat);
        for (ni, &(st, dur)) in rhythm.iter().enumerate() {
            let abs = p * 32 + st;
            if abs >= loop_steps {
                break;
            }
            let cd = chord_at(abs);
            let strong = st % 8 == 0;
            let last_note = ni == rhythm.len() - 1;
            let mut d;
            if repeat_phrase && ni < phrase_a.len() && !(last_phrase && last_note) {
                // repeat phrase A: literally, or as a diatonic sequence following the chord
                let (ad, acd) = phrase_a[ni];
                let seq = ad + (cd - acd);
                d = if rng.chance(style.literal) || seq < lo || seq > hi {
                    ad
                } else {
                    seq
                };
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
                if d < lo || d > hi {
                    // turn around at the edge of the range instead of jumping an octave
                    d = prev - step;
                }
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

// ------------------------------------------------------------------ melody composer
//
// Melodies are written the way the theory books describe (sentence and period forms,
// Narmour's implication-realization, non-chord tones that resolve by step), instead of
// wandering from chord tone to chord tone:
//
//  * a motif (basic idea) with its own rhythm and shape
//  * sentence: idea, idea repeated (sequenced to the new chord or kept), continuation
//    that fragments the idea and climbs, cadence on a long stable note
//  * period: idea + open ending (antecedent), idea again + closed ending (consequent)
//  * an arch contour: the phrase peaks once, about two thirds of the way through
//  * small intervals tend to continue, big leaps turn back by step (gap-fill)
//  * on strong beats: a chord tone, or a tension (9th, 11th, 6th) that resolves by step
//  * never an avoid note (a semitone above a chord tone) on a strong or long note,
//    never an augmented second, and its own register above the pad and keys

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Role {
    Free,
    Verse,
    Chorus,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fit {
    Chord,
    /// the chord's 7th: colour, fine on a strong beat, not as a final note
    Color,
    /// 9th, 11th, 13th that is not an avoid note
    Tension,
    /// a semitone above a chord tone
    Avoid,
    Out,
}

pub fn chord_pcs(key: i32, mode: usize, d: i32) -> [i32; 4] {
    [0, 2, 4, 6].map(|k| deg(key, mode, d + k).rem_euclid(12))
}

pub fn fit(n: i32, key: i32, mode: usize, d: i32) -> Fit {
    if !in_scale(key, mode, n) {
        return Fit::Out;
    }
    let pcs = chord_pcs(key, mode, d);
    let p = n.rem_euclid(12);
    if p == pcs[0] || p == pcs[1] || p == pcs[2] {
        return Fit::Chord;
    }
    if pcs[..3].iter().any(|&c| (p - c).rem_euclid(12) == 1) {
        return Fit::Avoid;
    }
    if p == pcs[3] {
        return Fit::Color;
    }
    Fit::Tension
}

/// Where the lead lives: above the pad (58-70) and the keys.
pub fn lead_range(role: Role) -> (i32, i32) {
    match role {
        Role::Verse => (69, 83),
        Role::Chorus => (70, 86),
        Role::Free => (67, 86),
    }
}

type Cell = &'static [(u32, u32)];

/// Basic-idea rhythms for a chorus or free lead (16 steps = one bar).
const HOOK_CELLS: &[Cell] = &[
    &[(0, 3), (3, 3), (6, 2), (8, 8)],
    &[(0, 2), (2, 2), (4, 4), (8, 4), (12, 4)],
    &[(2, 2), (4, 2), (6, 4), (10, 6)],
    &[(0, 4), (4, 2), (6, 6), (12, 4)],
    &[(0, 6), (6, 6), (12, 4)],
    &[(0, 2), (2, 4), (6, 2), (8, 8)],
];

/// Verse rhythms: room to breathe.
const VERSE_CELLS: &[Cell] = &[
    &[(0, 4), (4, 2), (6, 6)],
    &[(2, 2), (4, 4), (8, 4)],
    &[(0, 6), (8, 2), (10, 4)],
    &[(4, 4), (8, 8)],
    &[(0, 3), (3, 3), (6, 6)],
];

/// The open ending of a period's first half.
const OPEN_CELLS: &[Cell] = &[&[(0, 4), (4, 12)], &[(0, 2), (2, 2), (4, 12)], &[(0, 8), (8, 8)]];

/// Cadences: arrive and hold.
const CADENCE_CELLS: &[Cell] = &[
    &[(0, 12)],
    &[(0, 4), (4, 12)],
    &[(0, 2), (2, 2), (4, 10)],
    &[(0, 6), (6, 10)],
];
const VERSE_CADENCE_CELLS: &[Cell] = &[&[(0, 8)], &[(0, 4), (4, 8)], &[(0, 2), (2, 10)]];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Job {
    Idea,
    Repeat,
    Frag,
    Open,
    Parallel,
    Cadence,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    start: u32,
    dur: u32,
    job: Job,
    /// index within the copied material (for Repeat / Frag / Parallel)
    src: usize,
    /// for Frag: which copy (0, 1)
    copy: u32,
}

struct Composer<'a> {
    key: i32,
    mode: usize,
    prog: &'a [i32],
    bpc: u32,
    pool: Vec<i32>,
    role: Role,
    total: u32,
    temp: f32,
    tension: f32,
}

impl Composer<'_> {
    fn chord_at(&self, step: u32) -> i32 {
        self.prog[((step / 16 / self.bpc.max(1)) as usize) % self.prog.len()]
    }

    fn fit_at(&self, idx: usize, step: u32) -> Fit {
        fit(self.pool[idx], self.key, self.mode, self.chord_at(step))
    }

    /// Arch contour: low-middle start, peak around 65%, fall to the cadence.
    fn target(&self, step: u32) -> f32 {
        let t = step as f32 / self.total.max(1) as f32;
        let n = self.pool.len() as f32;
        let (base, peak) = match self.role {
            Role::Verse => (0.3, 0.72),
            _ => (0.3, 0.88),
        };
        let a = if t < 0.58 {
            t / 0.58
        } else {
            1.0 - 0.8 * (t - 0.58) / 0.42
        };
        n * (base + (peak - base) * a)
    }

    /// Score-and-sample the next note (index into the pool).
    #[allow(clippy::too_many_arguments)]
    fn choose(
        &self,
        rng: &mut Rng,
        prev: Option<usize>,
        prev_iv: i32,
        step: u32,
        dur: u32,
        must_resolve: bool,
        stable_end: bool,
        allow_tension: bool,
    ) -> usize {
        let strong = step % 8 == 0 || dur >= 6;
        let target = self.target(step);
        let lo = prev.map(|p| p.saturating_sub(6)).unwrap_or(0);
        let hi = prev
            .map(|p| (p + 6).min(self.pool.len() - 1))
            .unwrap_or(self.pool.len() - 1);
        let mut cands: Vec<(usize, f32)> = Vec::new();
        for c in lo..=hi {
            let f = self.fit_at(c, step);
            if f == Fit::Out {
                continue;
            }
            let n = self.pool[c];
            if must_resolve {
                // the tension before must step into a chord tone
                let Some(p) = prev else { continue };
                if (c as i32 - p as i32).abs() != 1 || f != Fit::Chord {
                    continue;
                }
            }
            let mut s = 0.0f32;
            s += if stable_end {
                match f {
                    Fit::Chord => {
                        let tonic = (n - self.key).rem_euclid(12) == 0;
                        if tonic {
                            5.0
                        } else {
                            3.0
                        }
                    }
                    _ => -20.0,
                }
            } else if strong {
                match f {
                    Fit::Chord => 3.0,
                    Fit::Color => 1.2,
                    Fit::Tension if allow_tension => 4.6,
                    Fit::Tension => -1.5,
                    Fit::Avoid => -12.0,
                    Fit::Out => -99.0,
                }
            } else {
                match f {
                    Fit::Chord => 1.0,
                    Fit::Color => 0.6,
                    Fit::Tension => 0.5,
                    Fit::Avoid if dur <= 2 => -1.0,
                    Fit::Avoid => -6.0,
                    Fit::Out => -99.0,
                }
            };
            if let Some(p) = prev {
                let pn = self.pool[p];
                let iv = n - pn;
                let a = iv.abs();
                let chorus = self.role != Role::Verse;
                s += match a {
                    0 => -0.8,
                    1 | 2 => 2.0,
                    3 | 4 => 1.0,
                    5 => 0.3,
                    7 => {
                        if chorus {
                            0.0
                        } else {
                            -0.8
                        }
                    }
                    6 => -3.0,
                    8 | 9 => {
                        if chorus {
                            -0.4
                        } else {
                            -1.2
                        }
                    }
                    12 => {
                        if chorus {
                            -0.3
                        } else {
                            -2.5
                        }
                    }
                    _ => -6.0,
                };
                let pa = prev_iv.abs();
                if pa >= 7 {
                    // a leap wants to turn back by step (gap-fill)
                    if a <= 4 && a > 0 && iv.signum() == -prev_iv.signum() {
                        s += 2.5;
                    } else if iv.signum() == prev_iv.signum() {
                        s -= 3.0;
                    }
                } else if (1..=4).contains(&pa) && (1..=4).contains(&a) && iv.signum() == prev_iv.signum() {
                    s += 0.6; // small intervals continue
                }
                if (c as i32 - p as i32).abs() == 1 && a == 3 {
                    s -= 8.0; // augmented second
                }
            }
            s -= 0.35 * (c as f32 - target).abs();
            cands.push((c, s));
        }
        if cands.is_empty() {
            // nothing fits the constraints: nearest chord tone to the target
            return (0..self.pool.len())
                .filter(|&c| self.fit_at(c, step) == Fit::Chord)
                .min_by(|&a, &b| (a as f32 - target).abs().total_cmp(&(b as f32 - target).abs()))
                .unwrap_or(self.pool.len() / 2);
        }
        let mx = cands.iter().map(|c| c.1).fold(f32::MIN, f32::max);
        let w: Vec<f32> = cands.iter().map(|c| ((c.1 - mx) / self.temp).exp()).collect();
        cands[rng.weighted(&w)].0
    }

    /// Like `fix`, but keeps the shape of the line: the replacement moves in the same
    /// direction from the previous note as the original did, and does not flatten a
    /// moving line into repeated notes.
    fn fix_dir(&self, idx: usize, step: u32, dur: u32, prev: Option<usize>, want: i32) -> usize {
        let fixed = self.fix(idx, step, dur);
        if fixed == idx {
            if let Some(p) = prev {
                if idx == p && want != 0 {
                    // a copy collapsed onto the previous note: nudge it the original way
                    for d in [want, 2 * want] {
                        let c = idx as i32 + d;
                        if c >= 0 && (c as usize) < self.pool.len() {
                            let f = self.fit_at(c as usize, step);
                            let strong = step % 8 == 0 || dur >= 6;
                            if f == Fit::Chord || (!strong && f != Fit::Avoid && f != Fit::Out) {
                                return c as usize;
                            }
                        }
                    }
                }
            }
            return idx;
        }
        let Some(p) = prev else { return fixed };
        let mut best = fixed;
        let mut bs = f32::MIN;
        for d in -3i32..=3 {
            let c = idx as i32 + d;
            if c < 0 || c as usize >= self.pool.len() || self.fit_at(c as usize, step) != Fit::Chord {
                continue;
            }
            let c = c as usize;
            let dir = (c as i32 - p as i32).signum();
            let mut sc = -(d.abs() as f32);
            if want != 0 && dir == want {
                sc += 2.0;
            }
            if c == p && want != 0 {
                sc -= 3.0;
            }
            if sc > bs {
                bs = sc;
                best = c;
            }
        }
        best
    }

    /// Make a copied note fit its new chord: strong or long notes that clash move to the
    /// nearest chord tone (keeping the direction of the original line where possible).
    fn fix(&self, idx: usize, step: u32, dur: u32) -> usize {
        let strong = step % 8 == 0 || dur >= 6;
        let f = self.fit_at(idx, step);
        let bad = match f {
            Fit::Out | Fit::Avoid => strong || dur > 2,
            Fit::Tension => strong,
            _ => false,
        };
        if !bad {
            return idx;
        }
        for d in [1i32, -1, 2, -2, 3, -3] {
            let c = idx as i32 + d;
            if c >= 0 && (c as usize) < self.pool.len() && self.fit_at(c as usize, step) == Fit::Chord {
                return c as usize;
            }
        }
        idx
    }
}

/// Compose a lead melody over one loop of the progression.
pub fn compose_melody(rng: &mut Rng, key: i32, mode: usize, prog: &[i32], bpc: u32, role: Role) -> Vec<LeadNote> {
    let bars = prog.len() as u32 * bpc.max(1);
    let unit = (bars * 4).max(16); // four units per loop
    let scale = unit / 16;
    let (lo, hi) = lead_range(role);
    let pool: Vec<i32> = (lo..=hi).filter(|&n| in_scale(key, mode, n)).collect();
    let c = Composer {
        key,
        mode,
        prog,
        bpc: bpc.max(1),
        pool,
        role,
        total: unit * 4,
        temp: if role == Role::Verse { 0.7 } else { 0.55 },
        tension: match role {
            Role::Verse => 0.3,
            Role::Chorus => 0.4,
            Role::Free => 0.35,
        },
    };
    let period = match role {
        Role::Chorus => rng.chance(0.5),
        Role::Free => rng.chance(0.35),
        Role::Verse => rng.chance(0.25),
    };
    let cells = if role == Role::Verse { VERSE_CELLS } else { HOOK_CELLS };
    let idea: Cell = cells[rng.below(cells.len())];
    let cad: Cell = if role == Role::Verse {
        VERSE_CADENCE_CELLS[rng.below(VERSE_CADENCE_CELLS.len())]
    } else {
        CADENCE_CELLS[rng.below(CADENCE_CELLS.len())]
    };
    let open: Cell = OPEN_CELLS[rng.below(OPEN_CELLS.len())];

    // ---- rhythm plan
    let mut slots: Vec<Slot> = Vec::new();
    let push = |slots: &mut Vec<Slot>, base: u32, cell: Cell, job: Job, copy: u32| {
        for (i, &(st, d)) in cell.iter().enumerate() {
            slots.push(Slot {
                start: base + st * scale,
                dur: d * scale,
                job,
                src: i,
                copy,
            });
        }
    };
    push(&mut slots, 0, idea, Job::Idea, 0);
    if period {
        push(&mut slots, unit, open, Job::Open, 0);
        push(&mut slots, unit * 2, idea, Job::Parallel, 0);
    } else {
        push(&mut slots, unit, idea, Job::Repeat, 0);
        // fragmentation: the first half of the idea, twice, climbing
        let half: Vec<(u32, u32)> = idea.iter().copied().filter(|&(st, _)| st < 8).collect();
        if half.len() >= 2 {
            for copy in 0..2u32 {
                for (i, &(st, d)) in half.iter().enumerate() {
                    let d = d.min(8 - st);
                    slots.push(Slot {
                        start: unit * 2 + (copy * 8 + st) * scale,
                        dur: d * scale,
                        job: Job::Frag,
                        src: i,
                        copy,
                    });
                }
            }
        } else {
            for k in 0..4u32 {
                slots.push(Slot {
                    start: unit * 2 + k * 4 * scale,
                    dur: 4 * scale,
                    job: Job::Frag,
                    src: 99,
                    copy: k,
                });
            }
        }
    }
    push(&mut slots, unit * 3, cad, Job::Cadence, 0);

    // ---- pitches
    let mut out: Vec<usize> = Vec::with_capacity(slots.len());
    let open_n = open.len();
    let root_step = |step: u32| -> i32 { c.chord_at(step) };
    let mut must_resolve = false;
    let mut prev_iv = 0i32;
    for k in 0..slots.len() {
        let sl = slots[k];
        let prev = out.last().copied();
        let next_close = slots
            .get(k + 1)
            .map(|n| n.start <= sl.start + sl.dur + 2)
            .unwrap_or(false);
        let last_of_job = slots
            .get(k + 1)
            .map(|n| n.job != sl.job || n.start >= unit * 3 && sl.start < unit * 3)
            .unwrap_or(true);
        let idx = match sl.job {
            Job::Idea | Job::Frag if sl.src == 99 => {
                let allow = rng.chance(c.tension) && next_close;
                c.choose(rng, prev, prev_iv, sl.start, sl.dur, must_resolve, false, allow)
            }
            Job::Idea => {
                let allow = rng.chance(c.tension) && next_close;
                c.choose(rng, prev, prev_iv, sl.start, sl.dur, must_resolve, false, allow)
            }
            Job::Repeat | Job::Parallel => {
                let src = out[sl.src];
                let want = if sl.src > 0 {
                    (out[sl.src] as i32 - out[sl.src - 1] as i32).signum()
                } else {
                    0
                };
                let shifted = if sl.job == Job::Repeat && rng.chance(0.6) {
                    // diatonic sequence: follow the chord's root movement
                    let mut k = (root_step(sl.start) - root_step(slots[sl.src].start)).rem_euclid(7);
                    if k > 3 {
                        k -= 7;
                    }
                    (src as i32 + k).clamp(0, c.pool.len() as i32 - 1) as usize
                } else {
                    src
                };
                c.fix_dir(shifted, sl.start, sl.dur, if sl.src > 0 { prev } else { None }, want)
            }
            Job::Frag => {
                let src = out[sl.src];
                let up = 1 + sl.copy as i32; // climbing
                let i = (src as i32 + up).clamp(0, c.pool.len() as i32 - 1) as usize;
                let want = if sl.src > 0 {
                    (out[sl.src] as i32 - out[sl.src - 1] as i32).signum()
                } else {
                    1
                };
                c.fix_dir(i, sl.start, sl.dur, prev, want)
            }
            Job::Open => {
                if last_of_job || sl.src + 1 == open_n {
                    // an open ending: a chord tone that is not the tonic
                    let mut best = prev.unwrap_or(c.pool.len() / 2);
                    let mut bs = f32::MIN;
                    for i in 0..c.pool.len() {
                        if c.fit_at(i, sl.start) != Fit::Chord || (c.pool[i] - key).rem_euclid(12) == 0 {
                            continue;
                        }
                        let d = prev.map(|p| (i as f32 - p as f32).abs()).unwrap_or(0.0);
                        let sc = -d + rng.f() * 0.5;
                        if sc > bs {
                            bs = sc;
                            best = i;
                        }
                    }
                    best
                } else {
                    c.choose(rng, prev, prev_iv, sl.start, sl.dur, must_resolve, false, false)
                }
            }
            Job::Cadence => {
                let is_last = k + 1 == slots.len();
                if is_last {
                    c.choose(rng, prev, prev_iv, sl.start, sl.dur, false, true, false)
                } else {
                    c.choose(rng, prev, prev_iv, sl.start, sl.dur, must_resolve, false, false)
                }
            }
        };
        // tension bookkeeping: a strong non-chord tone must resolve by step next
        let strong = sl.start % 8 == 0 || sl.dur >= 6;
        must_resolve = strong && matches!(c.fit_at(idx, sl.start), Fit::Tension) && next_close;
        if let Some(p) = prev {
            prev_iv = c.pool[idx] - c.pool[p];
        }
        out.push(idx);
    }

    // the cadence's last note is approached by step when there is a note before it
    let n = out.len();
    if n >= 2 && slots[n - 2].job == Job::Cadence {
        let fin = out[n - 1];
        let from_above = rng.chance(0.65);
        let cand = if from_above { fin + 1 } else { fin.saturating_sub(1) };
        if cand < c.pool.len() {
            let a = (c.pool[cand] - c.pool[fin]).abs();
            let f = c.fit_at(cand, slots[n - 2].start);
            if a != 3 && f != Fit::Avoid && f != Fit::Out {
                out[n - 2] = cand;
            }
        }
    }

    // ---- repair: whatever the copies produced, no avoid notes on strong or long notes
    // and no augmented seconds
    for _ in 0..3 {
        for k in 0..out.len() {
            let sl = slots[k];
            // a strong tension is fine when the next note resolves it by step (appoggiatura)
            let resolved = c.fit_at(out[k], sl.start) == Fit::Tension
                && slots.get(k + 1).is_some_and(|nx| {
                    nx.start <= sl.start + sl.dur + 2
                        && (out[k + 1] as i32 - out[k] as i32).abs() == 1
                        && c.fit_at(out[k + 1], nx.start) == Fit::Chord
                });
            if !resolved {
                let prev = if k > 0 { Some(out[k - 1]) } else { None };
                let want = prev.map(|p| (out[k] as i32 - p as i32).signum()).unwrap_or(0);
                out[k] = c.fix_dir(out[k], sl.start, sl.dur, prev, want);
            }
            if k > 0 {
                let (p, q) = (out[k - 1], out[k]);
                let a = (c.pool[q] - c.pool[p]).abs();
                if a == 3 && (q as i32 - p as i32).abs() == 1 {
                    // step past it: continue one more scale step, or turn back
                    let dir = q as i32 - p as i32;
                    let ahead = q as i32 + dir;
                    let alt = if ahead >= 0 && (ahead as usize) < c.pool.len() {
                        ahead as usize
                    } else {
                        p
                    };
                    let alt2 = (p as i32 - dir).clamp(0, c.pool.len() as i32 - 1) as usize;
                    let ok = |i: usize| !matches!(c.fit_at(i, sl.start), Fit::Avoid | Fit::Out);
                    out[k] = if ok(alt) { alt } else { alt2 };
                }
            }
        }
    }

    // ---- notes, with anticipations (the arrival pushed onto the "and" before the bar)
    let mut notes: Vec<LeadNote> = slots
        .iter()
        .zip(out.iter())
        .map(|(s, &i)| LeadNote {
            start: s.start,
            dur: s.dur,
            midi: c.pool[i],
        })
        .collect();
    if role != Role::Verse {
        for k in 1..notes.len() {
            let st = notes[k].start;
            if st % 16 == 0 && rng.chance(0.3) {
                let ns = st - 2;
                let p = notes[k - 1];
                // it must not clash with the chord it arrives early over
                let early = fit(notes[k].midi, key, mode, c.chord_at(ns));
                if p.start + 1 < ns && p.start + p.dur <= st && !matches!(early, Fit::Avoid | Fit::Out) {
                    notes[k - 1].dur = p.dur.min(ns - p.start);
                    notes[k].start = ns;
                    notes[k].dur += 2;
                }
            }
        }
    }
    notes
}

/// A progression from the template list that passes `keep` (only major/minor triads).
pub fn pick_prog_where(rng: &mut Rng, mode: usize, keep: impl Fn(&[i32]) -> bool) -> Option<Vec<i32>> {
    let v: Vec<&[i32]> = valid_progs(mode).into_iter().filter(|p| keep(p)).collect();
    if v.is_empty() {
        None
    } else {
        Some(v[rng.below(v.len())].to_vec())
    }
}

pub fn is_major_or_minor(mode: usize, d: i32) -> bool {
    matches!(triad_quality(mode, d), Quality::Maj | Quality::Min)
}

// ------------------------------------------------------------------ playing along

/// The engine keeps the tonic in MIDI 45..56; this maps a pitch class back to it.
pub fn key_root(pc: i32) -> i32 {
    45 + (pc - 9).rem_euclid(12)
}

pub fn in_scale(key: i32, mode: usize, n: i32) -> bool {
    MODES[mode].iv.contains(&(n - key).rem_euclid(12))
}

/// Absolute scale-degree index of an in-scale note (tonic of `key` = 0).
pub fn degree_of(key: i32, mode: usize, n: i32) -> Option<i32> {
    let rel = n - key;
    let pc = rel.rem_euclid(12);
    MODES[mode]
        .iv
        .iter()
        .position(|&x| x == pc)
        .map(|i| rel.div_euclid(12) * 7 + i as i32)
}

/// The notes offered on the phone's lead pads: two octaves of the scale.
pub fn lead_pads(key: i32, mode: usize) -> Vec<i32> {
    (0..15).map(|d| deg(key + 12, mode, d)).collect()
}

/// How likely each candidate is as the next melody note, given the chord, the last two
/// notes and the note the machine itself would play next. Returns probabilities (sum 1).
///
/// The rules are the ones melodies tend to follow: chord tones are stable, small steps
/// beat leaps, a leap is usually answered by a step back the other way, the leading tone
/// wants to rise to the tonic, the 4th likes to fall to the 3rd.
pub fn suggest_next(
    key: i32,
    mode: usize,
    chord_deg: i32,
    prev: Option<i32>,
    prev2: Option<i32>,
    machine: Option<i32>,
    cands: &[i32],
) -> Vec<f32> {
    let pc = |n: i32| n.rem_euclid(12);
    let chord: Vec<i32> = (0..4).map(|j| pc(deg(key, mode, chord_deg + 2 * j))).collect();
    let iv = &MODES[mode].iv;
    let mut w: Vec<f32> = cands
        .iter()
        .map(|&n| {
            let Some(dn) = degree_of(key, mode, n) else { return 0.0 };
            let mut x = 1.0f32;
            let p = pc(n);
            if p == chord[0] {
                x *= 3.2;
            } else if p == chord[1] || p == chord[2] {
                x *= 2.6;
            } else if p == chord[3] {
                x *= 1.4;
            }
            match prev.and_then(|q| degree_of(key, mode, q).map(|dq| (q, dq))) {
                Some((q, dq)) => {
                    let step = dn - dq;
                    x *= match step.abs() {
                        0 => 0.55,
                        1 => 1.9,
                        2 => 1.5,
                        3 => 0.9,
                        4 => 0.6,
                        7 => 0.5,
                        5 | 6 => 0.35,
                        _ => 0.12,
                    };
                    if let Some(dr) = prev2.and_then(|r| degree_of(key, mode, r)) {
                        let last = dq - dr;
                        if last.abs() >= 3 && step.signum() == -last.signum() && step.abs() <= 2 {
                            x *= 1.8;
                        }
                    }
                    let qd = dq.rem_euclid(7);
                    if qd == 6 && iv[6] == 11 && n == q + 1 {
                        x *= 2.5;
                    }
                    if qd == 3 && step == -1 {
                        x *= 1.5;
                    }
                }
                None => {}
            }
            let r = (n as f32 - 74.0) / 12.0;
            x *= (-0.5 * r * r).exp();
            if machine == Some(n) {
                x *= 2.4;
            }
            x
        })
        .collect();
    let sum: f32 = w.iter().sum();
    if sum > 0.0 {
        for x in w.iter_mut() {
            *x /= sum;
        }
    }
    w
}

// ------------------------------------------------------------------ keys comping

/// What a keys hit plays: the chord, or one tone of the broken-chord list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Chord,
    Tone(u8),
}

/// (step in bar, hit, length in steps, velocity)
pub type KeysPat = &'static [(u8, Hit, f32, f32)];

pub const KEYS_PATTERNS: [(&str, KeysPat); 7] = [
    ("ballad", &[(0, Hit::Chord, 15.0, 0.8)]),
    (
        "push",
        &[
            (0, Hit::Chord, 5.0, 0.85),
            (6, Hit::Chord, 3.0, 0.6),
            (10, Hit::Chord, 5.0, 0.7),
        ],
    ),
    (
        "3+3+2",
        &[
            (0, Hit::Chord, 5.0, 0.8),
            (6, Hit::Chord, 5.0, 0.65),
            (12, Hit::Chord, 3.5, 0.55),
        ],
    ),
    (
        "broken",
        &[
            (0, Hit::Tone(0), 4.0, 0.75),
            (2, Hit::Tone(1), 4.0, 0.5),
            (4, Hit::Tone(2), 4.0, 0.6),
            (6, Hit::Tone(3), 4.0, 0.5),
            (8, Hit::Tone(4), 4.0, 0.65),
            (10, Hit::Tone(3), 4.0, 0.5),
            (12, Hit::Tone(2), 4.0, 0.55),
            (14, Hit::Tone(1), 2.0, 0.5),
        ],
    ),
    (
        "stabs",
        &[
            (2, Hit::Chord, 1.2, 0.6),
            (6, Hit::Chord, 1.2, 0.55),
            (10, Hit::Chord, 1.2, 0.6),
            (14, Hit::Chord, 1.2, 0.55),
        ],
    ),
    (
        "two-and",
        &[
            (0, Hit::Chord, 3.0, 0.8),
            (4, Hit::Chord, 1.5, 0.45),
            (6, Hit::Chord, 2.0, 0.6),
            (10, Hit::Chord, 2.0, 0.65),
            (12, Hit::Chord, 3.0, 0.5),
        ],
    ),
    (
        "slow-rise",
        &[
            (0, Hit::Tone(0), 16.0, 0.7),
            (4, Hit::Tone(2), 12.0, 0.5),
            (8, Hit::Tone(3), 8.0, 0.55),
            (12, Hit::Tone(4), 4.0, 0.5),
        ],
    ),
];

/// Pick a comping pattern that suits the intensity.
pub fn pick_keys_pattern(rng: &mut Rng, intensity: f32) -> usize {
    let pool: &[usize] = if intensity < 0.4 {
        &[0, 6, 3, 2]
    } else if intensity < 0.75 {
        &[1, 2, 3, 5]
    } else {
        &[1, 4, 5]
    };
    *rng.pick(pool)
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
    fn suggestions_prefer_steps_and_chord_tones() {
        let key = key_root(9); // A
        let mode = 4; // aeolian
        let pads = lead_pads(key, mode);
        // previous note E5 (76) over Am: the strongest candidates should be near E5 and in Am
        let p = suggest_next(key, mode, 0, Some(76), Some(74), None, &pads);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        let best = pads[p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0];
        assert!((best - 76).abs() <= 4, "best {best}");
        let pc = best.rem_euclid(12);
        assert!([9, 0, 4].contains(&pc), "best {best} should be a chord tone of Am");
        // the machine's own next note gets a boost
        let q = suggest_next(key, mode, 0, Some(76), None, Some(79), &pads);
        let i79 = pads.iter().position(|&n| n == 79).unwrap();
        let r = suggest_next(key, mode, 0, Some(76), None, None, &pads);
        assert!(q[i79] > r[i79]);
        // out-of-scale notes never get suggested
        assert_eq!(suggest_next(key, mode, 0, None, None, None, &[70])[0], 0.0);
    }

    /// Melody quality report (run with --nocapture to see it).
    fn report(name: &str, gen: &dyn Fn(&mut Rng, i32, usize, &[i32], u32) -> Vec<LeadNote>) -> [f32; 8] {
        let mut acc = [0.0f32; 8];
        let mut nmel = 0.0;
        for s in 0..300u64 {
            let mut rng = Rng::new(s * 7 + 1);
            let key = key_root(rng.below(12) as i32);
            let dark = rng.f();
            let mode = pick_mode(&mut rng, dark);
            let prog = pick_prog(&mut rng, mode, 0.7);
            let bpc = if prog.len() == 2 { 2 } else { 1 + rng.below(2) as u32 };
            let notes = gen(&mut rng, key, mode, &prog, bpc);
            let chord_at = |st: u32| prog[((st / 16 / bpc) as usize) % prog.len()];
            let (mut strong, mut on_chord, mut tens_res, mut bad, mut aug2, mut leaps, mut recov, mut low) =
                (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            for (i, n) in notes.iter().enumerate() {
                let st = n.start % 16 == 0 || n.start % 16 == 8 || n.dur >= 6;
                let f = fit(n.midi, key, mode, chord_at(n.start));
                if n.midi <= 70 {
                    low += 1.0;
                }
                if st {
                    strong += 1.0;
                    match f {
                        Fit::Chord | Fit::Color => on_chord += 1.0,
                        Fit::Tension => {
                            if let Some(nx) = notes.get(i + 1) {
                                let a = (nx.midi - n.midi).abs();
                                if (1..=2).contains(&a) && fit(nx.midi, key, mode, chord_at(nx.start)) == Fit::Chord {
                                    tens_res += 1.0;
                                }
                            }
                        }
                        Fit::Avoid | Fit::Out => bad += 1.0,
                    }
                }
                if let Some(nx) = notes.get(i + 1) {
                    let iv = nx.midi - n.midi;
                    if iv.abs() == 3 {
                        let (a, b) = (n.midi.min(nx.midi), n.midi.max(nx.midi));
                        if (a + 1..b).all(|m| !in_scale(key, mode, m)) {
                            aug2 += 1.0;
                        }
                    }
                    if iv.abs() >= 7 {
                        leaps += 1.0;
                        if let Some(n2) = notes.get(i + 2) {
                            let iv2 = n2.midi - nx.midi;
                            if iv2.abs() <= 4 && iv2.signum() == -iv.signum() {
                                recov += 1.0;
                            }
                        }
                    }
                }
            }
            let nn = notes.len().max(1) as f32;
            let strong_n: f32 = if strong > 0.0 { strong } else { 1.0 };
            let peak = notes
                .iter()
                .enumerate()
                .max_by_key(|(_, n)| n.midi)
                .map(|(i, _)| i as f32 / nn)
                .unwrap_or(0.0);
            let row = [
                on_chord / strong_n,
                tens_res / strong_n,
                bad / strong_n,
                aug2,
                if leaps > 0.0 { recov / leaps } else { 1.0 },
                low / nn,
                peak,
                nn,
            ];
            for k in 0..8 {
                acc[k] += row[k];
            }
            nmel += 1.0;
        }
        for k in 0..8 {
            acc[k] /= nmel;
        }
        println!(
            "{name:<14} strong on chord {:>4.0}%  strong tension resolved {:>4.0}%  strong avoid/out {:>4.1}%  aug2/melody {:.2}  leaps recovered {:>4.0}%  notes in pad register {:>4.0}%  peak at {:>3.0}%  notes {:.1}",
            acc[0] * 100.0, acc[1] * 100.0, acc[2] * 100.0, acc[3], acc[4] * 100.0, acc[5] * 100.0, acc[6] * 100.0, acc[7]
        );
        acc
    }

    #[test]
    fn melody_examples() {
        let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        for (seed, role) in [
            (3u64, Role::Chorus),
            (8, Role::Chorus),
            (21, Role::Chorus),
            (5, Role::Verse),
            (13, Role::Verse),
        ] {
            let mut rng = Rng::new(seed);
            let key = key_root(9);
            let mode = 4;
            let prog = vec![5, 2, 6, 0];
            let m = compose_melody(
                &mut rng,
                key,
                mode,
                &prog,
                if role == Role::Verse { 2 } else { 1 },
                role,
            );
            let line: Vec<String> = m
                .iter()
                .map(|n| {
                    let f = fit(
                        n.midi,
                        key,
                        mode,
                        prog[((n.start / 16 / if role == Role::Verse { 2 } else { 1 }) as usize) % 4],
                    );
                    let tag = match f {
                        Fit::Tension => "*",
                        Fit::Color => "7",
                        _ => "",
                    };
                    format!(
                        "{}{}{}@{}:{}",
                        names[n.midi.rem_euclid(12) as usize],
                        n.midi / 12 - 1,
                        tag,
                        n.start,
                        n.dur
                    )
                })
                .collect();
            println!("{:?} seed {seed} over F C G Am: {}", role, line.join(" "));
        }
    }

    /// How much of the lead's time it rubs a semitone against a pad note (old pad and
    /// lead registers vs new): the "next to the note" sound.
    fn clash_report(
        name: &str,
        lead_shift: i32,
        clear: bool,
        gen: &dyn Fn(&mut Rng, i32, usize, &[i32], u32) -> Vec<LeadNote>,
    ) -> f32 {
        let (mut clash_t, mut total_t) = (0.0f32, 0.0f32);
        for s in 0..300u64 {
            let mut rng = Rng::new(s * 13 + 5);
            let key = key_root(rng.below(12) as i32);
            let dark = rng.f();
            let mode = pick_mode(&mut rng, dark);
            let prog = pick_prog(&mut rng, mode, 0.5);
            let bpc = if prog.len() == 2 { 2 } else { 1 + rng.below(2) as u32 };
            let notes = gen(&mut rng, key, mode, &prog, bpc);
            for n in &notes {
                let ci = ((n.start / 16 / bpc) as usize) % prog.len();
                let d = prog[ci];
                let span = (ci as u32 * bpc * 16, (ci as u32 + 1) * bpc * 16);
                let pcs: Vec<i32> = if clear {
                    notes
                        .iter()
                        .filter(|x| x.start >= span.0 && x.start < span.1)
                        .map(|x| x.midi.rem_euclid(12))
                        .collect()
                } else {
                    Vec::new()
                };
                let pad = voicing_clear(key + 12, mode, d, 5, 64.0, &pcs); // pad at verse intensity: 9th chords
                let m = n.midi + lead_shift;
                let rub = pad.iter().any(|&p| matches!(m - p, 1 | -1 | 11 | 13 | -11 | -13));
                total_t += n.dur as f32;
                if rub {
                    clash_t += n.dur as f32;
                }
            }
        }
        let r = clash_t / total_t;
        println!(
            "{name:<14} lead time rubbing a semitone (or 9th) against the pad: {:>4.1}%",
            r * 100.0
        );
        r
    }

    #[test]
    fn clash_with_pad() {
        clash_report("old verse", 0, false, &|r, k, m, p, b| {
            gen_lead_styled(r, k, m, p, b, &VERSE_STYLE)
        });
        clash_report("old chorus", 0, false, &|r, k, m, p, b| {
            gen_lead_styled(r, k, m, p, b, &CHORUS_STYLE)
        });
        clash_report("new verse", 0, false, &|r, k, m, p, b| {
            compose_melody(r, k, m, p, b, Role::Verse)
        });
        clash_report("new chorus", 0, false, &|r, k, m, p, b| {
            compose_melody(r, k, m, p, b, Role::Chorus)
        });
        let v = clash_report("new + clear v", 0, true, &|r, k, m, p, b| {
            compose_melody(r, k, m, p, b, Role::Verse)
        });
        let c = clash_report("new + clear c", 0, true, &|r, k, m, p, b| {
            compose_melody(r, k, m, p, b, Role::Chorus)
        });
        assert!(v < 0.2 && c < 0.2, "{v} {c}");
    }

    #[test]
    fn melody_report() {
        let old_v = report("old verse", &|r, k, m, p, b| {
            gen_lead_styled(r, k, m, p, b, &VERSE_STYLE)
        });
        let old_c = report("old chorus", &|r, k, m, p, b| {
            gen_lead_styled(r, k, m, p, b, &CHORUS_STYLE)
        });
        let new_v = report("new verse", &|r, k, m, p, b| compose_melody(r, k, m, p, b, Role::Verse));
        let new_c = report("new chorus", &|r, k, m, p, b| {
            compose_melody(r, k, m, p, b, Role::Chorus)
        });
        let _ = (old_v, old_c);
        for r in [new_v, new_c] {
            assert!(r[2] < 0.01, "avoid notes on strong beats: {}", r[2]);
            assert!(r[3] < 0.01, "augmented seconds: {}", r[3]);
            assert!(r[5] < 0.15, "too much in the pad register: {}", r[5]);
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
