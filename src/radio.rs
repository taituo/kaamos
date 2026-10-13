//! The radio: a shortwave receiver for song intros. Static, a tuning whistle, then
//! something comes through: morse, a numbers-station interval signal, or voices
//! (your own WAV clips, played as a crackly radio dialogue).

use std::sync::Arc;

use crate::dsp::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// just tuning around the band
    Tuning,
    /// a word in morse code
    Morse(String),
    /// a music-box interval signal (MIDI notes), played twice
    Signal(Vec<i32>),
    /// the loaded voice clips, one after another
    Voices,
}

impl Content {
    pub fn describe(&self, voices: usize) -> String {
        match self {
            Content::Tuning => "tuning static".into(),
            Content::Morse(w) => format!("morse \"{w}\""),
            Content::Signal(n) => format!("interval signal ({} notes of the chorus hook)", n.len()),
            Content::Voices => format!("{voices} voice clip{}", if voices == 1 { "" } else { "s" }),
        }
    }
}

const MORSE: [(char, &str); 36] = [
    ('A', ".-"),
    ('B', "-..."),
    ('C', "-.-."),
    ('D', "-.."),
    ('E', "."),
    ('F', "..-."),
    ('G', "--."),
    ('H', "...."),
    ('I', ".."),
    ('J', ".---"),
    ('K', "-.-"),
    ('L', ".-.."),
    ('M', "--"),
    ('N', "-."),
    ('O', "---"),
    ('P', ".--."),
    ('Q', "--.-"),
    ('R', ".-."),
    ('S', "..."),
    ('T', "-"),
    ('U', "..-"),
    ('V', "...-"),
    ('W', ".--"),
    ('X', "-..-"),
    ('Y', "-.--"),
    ('Z', "--.."),
    ('0', "-----"),
    ('1', ".----"),
    ('2', "..---"),
    ('3', "...--"),
    ('4', "....-"),
    ('5', "....."),
    ('6', "-...."),
    ('7', "--..."),
    ('8', "---.."),
    ('9', "----."),
];

const UNIT: f32 = 0.075; // seconds per morse unit (~16 wpm)

/// Morse beeps for a word as (start, length) in seconds, and the total length.
pub fn morse_beeps(word: &str) -> (Vec<(f32, f32)>, f32) {
    let mut t = 0.0;
    let mut out = Vec::new();
    for (wi, w) in word.split_whitespace().enumerate() {
        if wi > 0 {
            t += 4.0 * UNIT; // word gap is 7 units, 3 already added after the last letter
        }
        for c in w.chars() {
            let Some(code) = MORSE.iter().find(|(k, _)| *k == c.to_ascii_uppercase()).map(|x| x.1) else {
                continue;
            };
            for sym in code.chars() {
                let len = if sym == '-' { 3.0 * UNIT } else { UNIT };
                out.push((t, len));
                t += len + UNIT;
            }
            t += 2.0 * UNIT;
        }
    }
    (out, t)
}

/// A random five-character group, numbers-station style.
pub fn random_group(rng: &mut Rng) -> String {
    let cs = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    (0..5).map(|_| cs[rng.below(cs.len())] as char).collect()
}

const LEAD_IN: f32 = 2.2; // static + tuning before the content starts
const TAIL: f32 = 3.5; // static after the content

/// How long a radio passage lasts in seconds (voice clip lengths in seconds).
#[cfg(test)]
pub fn duration(c: &Content, voice_secs: &[f32]) -> f32 {
    LEAD_IN + content_len(c, voice_secs) + TAIL
}

/// When the content (morse, signal, voices) is over and the static starts to fade.
pub fn content_end(c: &Content, voice_secs: &[f32]) -> f32 {
    LEAD_IN + content_len(c, voice_secs)
}

fn content_len(c: &Content, voice_secs: &[f32]) -> f32 {
    match c {
        Content::Tuning => 3.0,
        Content::Morse(w) => 2.0 * morse_beeps(w).1 + 1.2,
        Content::Signal(n) => 2.0 * (n.len() as f32 * SIG_STEP + 0.6) + 1.0,
        Content::Voices => {
            if voice_secs.is_empty() {
                3.0
            } else {
                voice_secs.iter().map(|v| v + VOICE_GAP).sum()
            }
        }
    }
}

const SIG_STEP: f32 = 0.36;
const VOICE_GAP: f32 = 0.7;

#[derive(Clone, Copy)]
enum Ev {
    Beep(f32),
    Chime(f32),
    Voice(usize),
    Squelch,
}

pub struct Radio {
    sr: f32,
    active: bool,
    t: f32,
    dur: f32,
    content_end: f32,
    events: Vec<(f32, Ev)>,
    next_ev: usize,
    voices: Vec<Arc<Vec<f32>>>,
    // static
    noise: Noise,
    st_bp: Svf,
    st_lvl: f32,
    crackle: f32,
    crk_lp: OnePole,
    rng: Rng,
    // tuning whistle
    wh_ph: f32,
    wh_f: f32,
    // morse / chimes
    beep_left: f32,
    beep_ph: f32,
    beep_env: f32,
    chimes: [(f32, f32, f32); 4], // (phase, freq, amp)
    chime_next: usize,
    // voice playback
    voice: Option<(usize, usize)>,
    v_hp: Svf,
    v_lp1: Svf,
    v_lp2: Svf,
    squelch: f32,
    fade_ph: f32,
    tick: u32,
}

impl Radio {
    pub fn new(sr: f32) -> Self {
        Radio {
            sr,
            active: false,
            t: 0.0,
            dur: 0.0,
            content_end: 0.0,
            events: Vec::new(),
            next_ev: 0,
            voices: Vec::new(),
            noise: Noise::new(0x5ad10),
            st_bp: Svf::new(1800.0, 0.6, sr),
            st_lvl: 0.0,
            crackle: 0.0,
            crk_lp: OnePole::default(),
            rng: Rng::new(0xC0FFEE),
            wh_ph: 0.0,
            wh_f: 2400.0,
            beep_left: 0.0,
            beep_ph: 0.0,
            beep_env: 0.0,
            chimes: [(0.0, 0.0, 0.0); 4],
            chime_next: 0,
            voice: None,
            v_hp: Svf::new(330.0, 0.7, sr),
            v_lp1: Svf::new(3100.0, 0.7, sr),
            v_lp2: Svf::new(3100.0, 0.7, sr),
            squelch: 0.0,
            fade_ph: 0.0,
            tick: 0,
        }
    }

    pub fn set_voices(&mut self, v: Vec<Arc<Vec<f32>>>) {
        self.voices = v;
    }

    pub fn voice_secs(&self) -> Vec<f32> {
        self.voices.iter().map(|v| v.len() as f32 / self.sr).collect()
    }

    #[cfg(test)]
    pub fn active(&self) -> bool {
        self.active
    }

    /// Start a radio passage.
    pub fn start(&mut self, c: &Content) {
        let sr = self.sr;
        self.events.clear();
        let mut t = LEAD_IN;
        match c {
            Content::Tuning => {}
            Content::Morse(w) => {
                let (beeps, len) = morse_beeps(w);
                for rep in 0..2 {
                    for &(s, l) in &beeps {
                        self.events.push((t + s, Ev::Beep(l)));
                    }
                    t += len + if rep == 0 { 1.2 } else { 0.0 };
                }
            }
            Content::Signal(notes) => {
                for _ in 0..2 {
                    for &n in notes {
                        self.events.push((t, Ev::Chime(mtof(n as f32))));
                        t += SIG_STEP;
                    }
                    t += 0.6;
                }
            }
            Content::Voices => {
                for (i, v) in self.voices.iter().enumerate() {
                    self.events.push((t - 0.12, Ev::Squelch));
                    self.events.push((t, Ev::Voice(i)));
                    t += v.len() as f32 / sr;
                    self.events.push((t, Ev::Squelch));
                    t += VOICE_GAP;
                }
            }
        }
        self.content_end = content_end(c, &self.voice_secs());
        self.dur = self.content_end + TAIL;
        self.events.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.next_ev = 0;
        self.t = 0.0;
        self.wh_f = 2400.0;
        self.voice = None;
        self.active = true;
    }

    /// A short burst of static (for an outro or a bridge callback).
    pub fn start_static(&mut self, secs: f32) {
        self.events.clear();
        self.next_ev = 0;
        self.t = 0.0;
        self.content_end = 0.0;
        self.dur = secs.max(1.0);
        self.voice = None;
        self.active = true;
    }

    /// Returns (left, right, reverb send).
    #[inline]
    pub fn process(&mut self) -> (f32, f32, f32) {
        if !self.active {
            return (0.0, 0.0, 0.0);
        }
        let sr = self.sr;
        let dt = 1.0 / sr;
        let t = self.t;
        self.t += dt;
        if self.t > self.dur {
            self.active = false;
        }
        self.tick = self.tick.wrapping_add(1);

        while self.next_ev < self.events.len() && self.events[self.next_ev].0 <= t {
            match self.events[self.next_ev].1 {
                Ev::Beep(l) => self.beep_left = l,
                Ev::Chime(f) => {
                    self.chimes[self.chime_next] = (0.0, f, 1.0);
                    self.chime_next = (self.chime_next + 1) % self.chimes.len();
                }
                Ev::Voice(i) => self.voice = Some((i, 0)),
                Ev::Squelch => self.squelch = 0.12,
            }
            self.next_ev += 1;
        }

        // static: louder while tuning, ducked under the content, fading out at the end
        let in_content = t > LEAD_IN && t < self.content_end;
        let target = if t > self.dur - 2.5 {
            0.0
        } else if in_content {
            0.4
        } else {
            1.0
        };
        let rate = if t < 0.8 { 1.0 / (0.5 * sr) } else { 1.0 / (0.35 * sr) };
        self.st_lvl += (target - self.st_lvl) * rate;
        if self.tick % 64 == 0 {
            // the band drifts as if someone is turning the dial
            let wob = (t * 0.7).sin() * 0.5 + (t * 2.3).sin() * 0.2;
            self.st_bp.set(1700.0 * (wob).exp2(), 0.55, sr);
        }
        let n = self.noise.next();
        let hiss = self.st_bp.process(n).1 * 0.9;
        if self.rng.chance(0.0009) {
            self.crackle = self.rng.range(0.4, 1.0) * if self.rng.chance(0.5) { 1.0 } else { -1.0 };
        }
        self.crackle *= 0.93;
        let crk = self.crk_lp.lp(self.crackle, 0.35);
        let fading = 0.75 + 0.25 * (t * 1.9).sin() * (t * 0.37).sin();
        let mut out = (hiss + crk * 0.8) * self.st_lvl * fading * 0.3;

        // tuning whistle: a heterodyne gliding down while the dial turns
        if t < LEAD_IN + 0.3 {
            let target_f = 520.0 + 300.0 * (t * 3.1).sin();
            self.wh_f += (target_f - self.wh_f) * (2.2 / sr);
            self.wh_ph = (self.wh_ph + self.wh_f / sr).fract();
            let env = smoothstep(0.1, 0.6, t) * (1.0 - smoothstep(LEAD_IN - 0.6, LEAD_IN + 0.3, t));
            out += (self.wh_ph * TAU).sin() * env * 0.07;
        }

        // content
        let mut content = 0.0;
        if self.beep_left > 0.0 {
            self.beep_left -= dt;
            self.beep_env += (1.0 - self.beep_env) * 0.01;
        } else {
            self.beep_env *= 0.99;
        }
        if self.beep_env > 1e-4 {
            self.beep_ph = (self.beep_ph + 720.0 / sr).fract();
            content += (self.beep_ph * TAU).sin() * self.beep_env * 0.32;
        }
        for c in self.chimes.iter_mut() {
            if c.2 > 1e-4 {
                c.0 = (c.0 + c.1 / sr).fract();
                let x = c.0 * TAU;
                content += (x.sin() + 0.3 * (2.0 * x).sin() + 0.12 * (3.0 * x).sin()) * c.2 * 0.22;
                c.2 *= (-1.0 / (0.7 * sr)).exp();
            }
        }
        if let Some((i, pos)) = self.voice {
            let clip = &self.voices[i];
            if pos < clip.len() {
                // alternate the colour so a dialogue sounds like two different radios
                let (drive, flutter) = if i % 2 == 0 { (2.6, 0.18) } else { (3.6, 0.28) };
                let x = clip[pos];
                let x = self.v_hp.process(x).2;
                let x = self.v_lp2.process(self.v_lp1.process(x).0).0;
                let x = tanh_fast(x * drive) / tanh_fast(drive);
                self.fade_ph = (self.fade_ph + 1.3 / sr).fract();
                let am = 1.0 - flutter * (0.5 + 0.5 * (self.fade_ph * TAU).sin());
                content += x * am * 0.6;
                self.voice = Some((i, pos + 1));
            } else {
                self.voice = None;
            }
        }
        if self.squelch > 0.0 {
            self.squelch -= dt;
            out += self.noise.next() * 0.16;
        }
        out += content;
        let p = if let Some((i, _)) = self.voice {
            if i % 2 == 0 {
                -0.25
            } else {
                0.25
            }
        } else {
            0.0
        };
        let (l, r) = pan(out, p);
        (l, r, out * 0.25)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morse_timing() {
        // "E" is one dot: one unit on, then the letter gap
        let (b, len) = morse_beeps("E");
        assert_eq!(b.len(), 1);
        assert!((b[0].1 - UNIT).abs() < 1e-6);
        assert!((len - 4.0 * UNIT).abs() < 1e-5);
        // KAAMOS = -.- .- .- -- --- ...  -> 3+2+2+2+3+3 = 15 beeps
        assert_eq!(morse_beeps("KAAMOS").0.len(), 15);
    }

    #[test]
    fn radio_runs_and_ends() {
        let sr = 8000.0;
        let mut r = Radio::new(sr);
        r.set_voices(vec![Arc::new(vec![0.5; 4000]), Arc::new(vec![-0.5; 2000])]);
        let c = Content::Voices;
        let d = duration(&c, &r.voice_secs());
        r.start(&c);
        let mut n = 0;
        let mut peak = 0.0f32;
        while r.active() {
            let (l, rr, _) = r.process();
            peak = peak.max(l.abs()).max(rr.abs());
            n += 1;
            assert!(n < (sr * 60.0) as usize);
        }
        assert!(((n as f32 / sr) - d).abs() < 0.05, "{} vs {d}", n as f32 / sr);
        assert!(peak > 0.05 && peak < 1.0, "peak {peak}");
    }
}
