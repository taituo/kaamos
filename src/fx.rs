//! Effects: Juno-style chorus, tape echo, FDN hall reverb, gated reverb, tape machine, limiter.

use crate::dsp::*;

// ---------------------------------------------------------------- chorus

/// Mono in, stereo out. Two BBD-style modulated delays with inverted LFOs.
pub struct Chorus {
    dl: DelayLine,
    ph: f32,
    ph2: f32,
    sr: f32,
}

impl Chorus {
    pub fn new(sr: f32) -> Self {
        Chorus {
            dl: DelayLine::new((sr * 0.03) as usize),
            ph: 0.0,
            ph2: 0.25,
            sr,
        }
    }
    #[inline]
    pub fn process(&mut self, x: f32, depth: f32) -> (f32, f32) {
        self.dl.write(x);
        let tri = |p: f32| 1.0 - 4.0 * (p - 0.5).abs();
        let pr = (self.ph + 0.5).fract();
        let wob = (self.ph2 * TAU).sin();
        let base = 0.0058 * self.sr;
        let dep = 0.0024 * self.sr * depth;
        let wl = self.dl.read(base + dep * tri(self.ph) + 0.0004 * self.sr * wob);
        let wr = self.dl.read(base + dep * tri(pr) - 0.0004 * self.sr * wob);
        self.ph = (self.ph + 0.47 / self.sr).fract();
        self.ph2 = (self.ph2 + 0.11 / self.sr).fract();
        (0.72 * x + 0.72 * wl, 0.72 * x + 0.72 * wr)
    }
}

// ---------------------------------------------------------------- tape echo

/// Ping-pong echo with darkening, slightly saturated feedback.
pub struct Echo {
    l: DelayLine,
    r: DelayLine,
    t: f32,
    lp_l: OnePole,
    lp_r: OnePole,
    hp_l: OnePole,
    hp_r: OnePole,
    a_lp: f32,
    a_hp: f32,
    sr: f32,
}

impl Echo {
    pub fn clear(&mut self) {
        self.l.clear();
        self.r.clear();
    }

    pub fn new(sr: f32) -> Self {
        Echo {
            l: DelayLine::new((sr * 2.1) as usize),
            r: DelayLine::new((sr * 2.1) as usize),
            t: sr * 0.4,
            lp_l: OnePole::default(),
            lp_r: OnePole::default(),
            hp_l: OnePole::default(),
            hp_r: OnePole::default(),
            a_lp: onepole_a(2600.0, sr),
            a_hp: onepole_a(180.0, sr),
            sr,
        }
    }
    /// time in seconds, fb 0..0.9
    #[inline]
    pub fn process(&mut self, x: f32, time: f32, fb: f32) -> (f32, f32) {
        let target = (time * self.sr).clamp(10.0, self.sr * 2.0);
        self.t += (target - self.t) * 0.00004;
        let ol = self.l.read(self.t);
        let or = self.r.read(self.t);
        let fl = self.hp_l.hp(self.lp_l.lp(ol, self.a_lp), self.a_hp);
        let fr = self.hp_r.hp(self.lp_r.lp(or, self.a_lp), self.a_hp);
        self.l.write(x + tanh_fast(fr * fb));
        self.r.write(tanh_fast(fl * fb));
        (ol, or)
    }
}

// ---------------------------------------------------------------- reverb

/// 8-line feedback delay network with input diffusion and gentle modulation.
pub struct Reverb {
    lines: Vec<DelayLine>,
    lens: [f32; 8],
    gains: [f32; 8],
    damp: [OnePole; 8],
    damp_a: f32,
    pre: DelayLine,
    pre_len: usize,
    diff: Vec<Allpass>,
    lfo: f32,
    sr: f32,
    t60: f32,
}

const FDN_MS: [f32; 8] = [43.7, 51.3, 59.9, 67.1, 73.3, 83.9, 91.7, 101.9];

impl Reverb {
    pub fn clear(&mut self) {
        for l in self.lines.iter_mut() {
            l.clear();
        }
        self.pre.clear();
    }

    pub fn new(sr: f32, size: f32, pre_ms: f32, damp_hz: f32) -> Self {
        let mut lens = [0.0; 8];
        let mut lines = Vec::new();
        for i in 0..8 {
            lens[i] = FDN_MS[i] * 0.001 * size * sr;
            lines.push(DelayLine::new(lens[i] as usize + 64));
        }
        let s = sr / 44100.0;
        let diff = [142.0, 107.0, 379.0, 277.0]
            .iter()
            .map(|&n| Allpass::new(((n * s * size.sqrt()) as usize).max(8), 0.65))
            .collect();
        let pre_len = ((pre_ms * 0.001 * sr) as usize).max(1);
        let mut r = Reverb {
            lines,
            lens,
            gains: [0.0; 8],
            damp: [OnePole::default(); 8],
            damp_a: onepole_a(damp_hz, sr),
            pre: DelayLine::new(pre_len + 4),
            pre_len,
            diff,
            lfo: 0.0,
            sr,
            t60: 0.0,
        };
        r.set_t60(4.0);
        r
    }

    pub fn set_t60(&mut self, t60: f32) {
        if (t60 - self.t60).abs() < 0.01 {
            return;
        }
        self.t60 = t60;
        for i in 0..8 {
            self.gains[i] = 10f32.powf(-3.0 * self.lens[i] / (t60.max(0.1) * self.sr));
        }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.pre.write((l + r) * 0.5);
        let mut x = self.pre.tap(self.pre_len);
        for ap in self.diff.iter_mut() {
            x = ap.process(x);
        }
        let side = (l - r) * 0.3;
        self.lfo = (self.lfo + 0.37 / self.sr).fract();
        let m = (self.lfo * TAU).sin() * 0.0009 * self.sr;
        let mut y = [0.0f32; 8];
        for i in 0..8 {
            let d = match i {
                1 => self.lens[i] + m,
                6 => self.lens[i] - m,
                _ => self.lens[i],
            };
            y[i] = self.damp[i].lp(self.lines[i].read(d), self.damp_a);
        }
        // Fast Walsh–Hadamard transform (orthogonal mix).
        let mut h = y;
        let mut span = 1;
        while span < 8 {
            let mut i = 0;
            while i < 8 {
                for j in i..i + span {
                    let a = h[j];
                    let b = h[j + span];
                    h[j] = a + b;
                    h[j + span] = a - b;
                }
                i += span * 2;
            }
            span *= 2;
        }
        const NORM: f32 = 0.353_553_4; // 1/sqrt(8)
        for i in 0..8 {
            let inp = if i % 2 == 0 { x + side } else { -x + side * 0.5 };
            self.lines[i].write(h[i] * NORM * self.gains[i] + inp * 0.5);
        }
        let ol = y[0] - y[2] + y[4] - y[6] + 0.5 * (y[1] - y[5]);
        let or = y[1] - y[3] + y[5] - y[7] + 0.5 * (y[2] - y[6]);
        (ol * 0.42, or * 0.42)
    }
}

// ---------------------------------------------------------------- tape machine

/// Saturation, wow & flutter, high-frequency loss and hiss. amount 0..1.
pub struct Tape {
    dl: [DelayLine; 2],
    wow: f32,
    flutter: f32,
    drift: f32,
    drift_target: f32,
    drift_timer: u32,
    lp: [OnePole; 2],
    lp2: [OnePole; 2],
    hiss: Noise,
    hiss_lp: [OnePole; 2],
    hiss_hp: [OnePole; 2],
    rng: Rng,
    sr: f32,
}

impl Tape {
    pub fn new(sr: f32, seed: u64) -> Self {
        Tape {
            dl: [
                DelayLine::new((sr * 0.03) as usize),
                DelayLine::new((sr * 0.03) as usize),
            ],
            wow: 0.0,
            flutter: 0.0,
            drift: 0.0,
            drift_target: 0.0,
            drift_timer: 0,
            lp: [OnePole::default(); 2],
            lp2: [OnePole::default(); 2],
            hiss: Noise::new(seed as u32 ^ 0xA5A5),
            hiss_lp: [OnePole::default(); 2],
            hiss_hp: [OnePole::default(); 2],
            rng: Rng::new(seed ^ 77),
            sr,
        }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32, amt: f32) -> (f32, f32) {
        let sr = self.sr;
        // saturation (unity for small signals)
        let d = 1.0 + 1.1 * amt;
        let l = tanh_fast(l * d) / d;
        let r = tanh_fast(r * d) / d;

        // wow & flutter with slow random drift
        if self.drift_timer == 0 {
            self.drift_target = self.rng.range(-1.0, 1.0);
            self.drift_timer = (sr * self.rng.range(0.7, 2.5)) as u32;
        }
        self.drift_timer -= 1;
        self.drift += (self.drift_target - self.drift) * (1.6 / sr);
        self.wow = (self.wow + 0.31 / sr).fract();
        self.flutter = (self.flutter + 5.7 / sr).fract();
        let depth = amt * amt.sqrt();
        let mod_s =
            depth * (0.0021 * (self.wow * TAU).sin() + 0.00008 * (self.flutter * TAU).sin() + 0.0009 * self.drift);
        let delay = (0.008 + mod_s) * sr;
        self.dl[0].write(l);
        self.dl[1].write(r);
        let mut ol = self.dl[0].read(delay);
        let mut or = self.dl[1].read(delay);

        // high-frequency loss
        let a = onepole_a(17000.0 - 9500.0 * amt, sr);
        ol = self.lp2[0].lp(self.lp[0].lp(ol, a), a);
        or = self.lp2[1].lp(self.lp[1].lp(or, a), a);

        // hiss
        if amt > 0.001 {
            let lvl = db(-66.0 + 16.0 * amt) * amt.sqrt();
            let a_l = onepole_a(7000.0, sr);
            let a_h = onepole_a(900.0, sr);
            let n1 = self.hiss.next();
            let n2 = self.hiss.next();
            let h1 = self.hiss_hp[0].hp(self.hiss_lp[0].lp(n1, a_l), a_h);
            let h2 = self.hiss_hp[1].hp(self.hiss_lp[1].lp(n2, a_l), a_h);
            ol += h1 * lvl;
            or += h2 * lvl;
        }
        (ol, or)
    }
}

// ---------------------------------------------------------------- limiter

pub struct Limiter {
    env: f32,
    gain: f32,
    rel: f32,
    g_att: f32,
    g_rel: f32,
    dl: [DelayLine; 2],
    la: usize,
    thr: f32,
}

impl Limiter {
    /// Look-ahead peak limiter: peaks are seen 4 ms before they leave the delay line and
    /// the gain is already down when they arrive, so the safety clip never has to act.
    pub fn new(sr: f32, thr_db: f32) -> Self {
        let la = (sr * 0.004) as usize;
        Limiter {
            env: 0.0,
            gain: 1.0,
            rel: coef(0.25, sr),
            g_att: coef(0.0008, sr),
            g_rel: coef(0.06, sr),
            dl: [DelayLine::new(la + 4), DelayLine::new(la + 4)],
            la,
            thr: db(thr_db),
        }
    }
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let p = l.abs().max(r.abs());
        if p > self.env {
            self.env = p; // instant: the look-ahead gives the gain time to follow
        } else {
            self.env += (p - self.env) * self.rel;
        }
        let target = if self.env > self.thr { self.thr / self.env } else { 1.0 };
        let c = if target < self.gain { self.g_att } else { self.g_rel };
        self.gain += (target - self.gain) * c;
        self.dl[0].write(l);
        self.dl[1].write(r);
        let ol = self.dl[0].tap(self.la) * self.gain;
        let or = self.dl[1].tap(self.la) * self.gain;
        (ol.clamp(-0.985, 0.985), or.clamp(-0.985, 0.985))
    }
}
