//! Low-level DSP building blocks. Everything here is plain std Rust.

use std::f32::consts::PI;

pub const TAU: f32 = 2.0 * PI;

#[inline]
pub fn mtof(m: f32) -> f32 {
    440.0 * ((m - 69.0) / 12.0).exp2()
}

#[inline]
pub fn db(d: f32) -> f32 {
    10f32.powf(d / 20.0)
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn flush(x: f32) -> f32 {
    if x.abs() < 1e-18 {
        0.0
    } else {
        x
    }
}

/// Cheap rational tanh, accurate enough for saturation.
#[inline]
pub fn tanh_fast(x: f32) -> f32 {
    if x > 3.0 {
        1.0
    } else if x < -3.0 {
        -1.0
    } else {
        let x2 = x * x;
        x * (27.0 + x2) / (27.0 + 9.0 * x2)
    }
}

/// Constant-power pan, p in -1..1.
#[inline]
pub fn pan(x: f32, p: f32) -> (f32, f32) {
    let a = (p.clamp(-1.0, 1.0) + 1.0) * 0.25 * PI;
    (x * a.cos(), x * a.sin())
}

/// One-pole smoothing coefficient for a time constant in seconds at a given update rate.
#[inline]
pub fn coef(tau: f32, rate: f32) -> f32 {
    1.0 - (-1.0 / (tau.max(1e-5) * rate)).exp()
}

// ---------------------------------------------------------------- random

#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut r = Rng(seed ^ 0x2545_F491_4F6C_DD1D);
        r.next();
        r
    }
    #[inline]
    pub fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    #[inline]
    pub fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    #[inline]
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    pub fn pick<'a, T>(&mut self, s: &'a [T]) -> &'a T {
        &s[self.below(s.len())]
    }
    /// Pick an index with the given (non-negative) weights.
    pub fn weighted(&mut self, w: &[f32]) -> usize {
        let total: f32 = w.iter().sum();
        if total <= 0.0 {
            return self.below(w.len());
        }
        let mut r = self.f() * total;
        for (i, &x) in w.iter().enumerate() {
            if r < x {
                return i;
            }
            r -= x;
        }
        w.len() - 1
    }
}

/// Fast audio-rate white noise.
#[derive(Clone)]
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Noise(seed | 1)
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

// ---------------------------------------------------------------- oscillators

#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Default)]
pub struct Osc {
    pub phase: f32,
}

impl Osc {
    pub fn with_phase(p: f32) -> Self {
        Osc { phase: p.fract() }
    }
    #[inline]
    fn adv(&mut self, dt: f32) {
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
    }
    /// Band-limited sawtooth. dt = freq / sample_rate.
    #[inline]
    pub fn saw(&mut self, dt: f32) -> f32 {
        let dt = dt.min(0.45);
        let p = self.phase;
        let y = 2.0 * p - 1.0 - blep(p, dt);
        self.adv(dt);
        y
    }
    /// Band-limited pulse with width pw (0..1).
    #[inline]
    pub fn pulse(&mut self, dt: f32, pw: f32) -> f32 {
        let dt = dt.min(0.45);
        let pw = pw.clamp(0.05, 0.95);
        let p = self.phase;
        let mut y = if p < pw { 1.0 } else { -1.0 };
        y += blep(p, dt);
        let mut p2 = p - pw;
        if p2 < 0.0 {
            p2 += 1.0;
        }
        y -= blep(p2, dt);
        self.adv(dt);
        y
    }
    #[inline]
    pub fn sine(&mut self, dt: f32) -> f32 {
        let y = (self.phase * TAU).sin();
        self.adv(dt);
        y
    }
}

// ---------------------------------------------------------------- filters

/// 4-pole zero-delay-feedback ladder low-pass with input saturation.
/// k = resonance 0..~3.8 (self-oscillation near 4).
#[derive(Clone, Copy, Default)]
pub struct Ladder {
    s: [f32; 4],
}

impl Ladder {
    #[inline]
    pub fn process(&mut self, x: f32, cutoff: f32, k: f32, sr: f32) -> f32 {
        let fc = cutoff.clamp(16.0, sr * 0.42);
        let g = (PI * fc / sr).tan();
        let gg = g / (1.0 + g);
        let h = 1.0 - gg;
        let g2 = gg * gg;
        let g3 = g2 * gg;
        let g4 = g3 * gg;
        let s = &mut self.s;
        let sum = g3 * h * s[0] + g2 * h * s[1] + gg * h * s[2] + h * s[3];
        let xin = x * (1.0 + 0.6 * k);
        let y4 = (g4 * xin + sum) / (1.0 + k * g4);
        let mut u = tanh_fast(xin - k * y4);
        for st in s.iter_mut() {
            let v = (u - *st) * gg;
            let y = v + *st;
            *st = flush(y + v);
            u = y;
        }
        u
    }
    pub fn reset(&mut self) {
        self.s = [0.0; 4];
    }
}

/// Cytomic / Simper state-variable filter.
#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    pub fn new(fc: f32, q: f32, sr: f32) -> Self {
        let mut s = Svf::default();
        s.set(fc, q, sr);
        s
    }
    #[inline]
    pub fn set(&mut self, fc: f32, q: f32, sr: f32) {
        let g = (PI * fc.clamp(10.0, sr * 0.45) / sr).tan();
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    /// Returns (low, band, high).
    #[inline]
    pub fn process(&mut self, v0: f32) -> (f32, f32, f32) {
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        (v2, v1, v0 - self.k * v1 - v2)
    }
}

#[derive(Clone, Copy, Default)]
pub struct OnePole {
    pub y: f32,
}

impl OnePole {
    #[inline]
    pub fn lp(&mut self, x: f32, a: f32) -> f32 {
        self.y = flush(self.y + a * (x - self.y));
        self.y
    }
    #[inline]
    pub fn hp(&mut self, x: f32, a: f32) -> f32 {
        x - self.lp(x, a)
    }
}

#[inline]
pub fn onepole_a(fc: f32, sr: f32) -> f32 {
    1.0 - (-TAU * fc / sr).exp()
}

#[derive(Clone, Copy, Default)]
pub struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.9995 * self.y1;
        self.x1 = x;
        self.y1 = flush(y);
        y
    }
}

// ---------------------------------------------------------------- envelopes

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Analog-style exponential ADSR.
#[derive(Clone, Copy)]
pub struct Adsr {
    stage: Stage,
    pub level: f32,
    ac: f32,
    dc: f32,
    rc: f32,
    sustain: f32,
}

impl Adsr {
    pub fn new(a: f32, d: f32, s: f32, r: f32, sr: f32) -> Self {
        let mut e = Adsr {
            stage: Stage::Idle,
            level: 0.0,
            ac: 0.0,
            dc: 0.0,
            rc: 0.0,
            sustain: s,
        };
        e.set(a, d, s, r, sr);
        e
    }
    pub fn set(&mut self, a: f32, d: f32, s: f32, r: f32, sr: f32) {
        self.ac = coef(a / 1.5, sr);
        self.dc = coef(d, sr);
        self.rc = coef(r, sr);
        self.sustain = s;
    }
    pub fn set_release(&mut self, r: f32, sr: f32) {
        self.rc = coef(r, sr);
    }
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }
    pub fn active(&self) -> bool {
        self.stage != Stage::Idle
    }
    pub fn releasing(&self) -> bool {
        self.stage == Stage::Release
    }
    #[inline]
    pub fn tick(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += (1.25 - self.level) * self.ac;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level += (self.sustain - self.level) * self.dc;
                if (self.level - self.sustain).abs() < 1e-4 {
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = self.sustain,
            Stage::Release => {
                self.level -= self.level * self.rc;
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// Simple exponential decay envelope (for percussion / filter env).
#[derive(Clone, Copy, Default)]
pub struct Decay {
    pub v: f32,
    c: f32,
}

impl Decay {
    pub fn trigger(&mut self, amount: f32, tau: f32, sr: f32) {
        self.v = amount;
        self.c = (-1.0 / (tau.max(1e-4) * sr)).exp();
    }
    #[inline]
    pub fn tick(&mut self) -> f32 {
        let v = self.v;
        self.v = flush(self.v * self.c);
        v
    }
}

// ---------------------------------------------------------------- delay lines

pub struct DelayLine {
    buf: Vec<f32>,
    mask: usize,
    w: usize,
}

impl DelayLine {
    pub fn clear(&mut self) {
        self.buf.iter_mut().for_each(|x| *x = 0.0);
    }

    pub fn new(max_samples: usize) -> Self {
        let n = (max_samples + 4).next_power_of_two();
        DelayLine {
            buf: vec![0.0; n],
            mask: n - 1,
            w: 0,
        }
    }
    #[inline]
    pub fn write(&mut self, x: f32) {
        self.w = (self.w + 1) & self.mask;
        self.buf[self.w] = flush(x);
    }
    /// Integer read, d >= 1 samples ago... d=0 is the most recently written.
    #[inline]
    pub fn tap(&self, d: usize) -> f32 {
        self.buf[(self.w.wrapping_sub(d)) & self.mask]
    }
    /// Fractional read with cubic (Hermite) interpolation.
    #[inline]
    pub fn read(&self, d: f32) -> f32 {
        let d = d.clamp(1.0, (self.mask - 3) as f32);
        let i = d as usize;
        let f = d - i as f32;
        let y0 = self.tap(i - 1);
        let y1 = self.tap(i);
        let y2 = self.tap(i + 1);
        let y3 = self.tap(i + 2);
        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * f + c2) * f + c1) * f + c0
    }
}

/// Schroeder allpass diffuser.
pub struct Allpass {
    dl: DelayLine,
    len: usize,
    g: f32,
}

impl Allpass {
    pub fn new(len: usize, g: f32) -> Self {
        Allpass {
            dl: DelayLine::new(len + 2),
            len,
            g,
        }
    }
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let d = self.dl.tap(self.len - 1);
        let v = x + self.g * d;
        self.dl.write(v);
        d - self.g * v
    }
}
