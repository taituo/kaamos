//! All instruments, synthesized from scratch: pad, arp, bass, drone, lead, bells, drums, impacts.

use crate::dsp::*;

// ================================================================ PAD
// Juno-style poly pad: 5 detuned saws + PWM pulse (+ optional octave shimmer)
// through a 24 dB ladder per voice. Width comes later from the chorus.

const PAD_CENTS: [f32; 5] = [-11.0, -5.0, 0.0, 5.5, 10.5];

struct PadVoice {
    osc: [Osc; 5],
    hi: Osc,
    pw: Osc,
    ratio: [f32; 5],
    filt: Ladder,
    env: Adsr,
    note: f32,
    vel: f32,
    age: u32,
}

pub struct Pad {
    v: Vec<PadVoice>,
    sr: f32,
    pwm: f32,
    rng: Rng,
    counter: u32,
}

impl Pad {
    pub fn new(sr: f32, seed: u64) -> Self {
        let v = (0..14)
            .map(|_| PadVoice {
                osc: [Osc::default(); 5],
                hi: Osc::default(),
                pw: Osc::default(),
                ratio: [1.0; 5],
                filt: Ladder::default(),
                env: Adsr::new(2.0, 1.5, 0.85, 3.0, sr),
                note: 60.0,
                vel: 0.0,
                age: 0,
            })
            .collect();
        Pad {
            v,
            sr,
            pwm: 0.0,
            rng: Rng::new(seed ^ 0x9AD),
            counter: 0,
        }
    }

    fn alloc(&self) -> usize {
        if let Some(i) = self.v.iter().position(|v| !v.env.active()) {
            return i;
        }
        // steal the quietest releasing voice, else the oldest
        let mut best = 0;
        let mut best_score = f32::MAX;
        for (i, v) in self.v.iter().enumerate() {
            let score = if v.env.releasing() {
                v.env.level
            } else {
                10.0 + v.age as f32 * 1e-6
            };
            if score < best_score {
                best_score = score;
                best = i;
            }
        }
        best
    }

    pub fn note_on(&mut self, note: i32, vel: f32, attack: f32, release: f32, spread: f32) {
        let i = self.alloc();
        let idle = !self.v[i].env.active();
        let sr = self.sr;
        let counter = self.counter;
        self.counter = self.counter.wrapping_add(1);
        let rng = &mut self.rng;
        let v = &mut self.v[i];
        if idle {
            for o in v.osc.iter_mut() {
                o.phase = rng.f();
            }
            v.hi.phase = rng.f();
            v.pw.phase = rng.f();
            v.filt.reset();
        }
        for k in 0..5 {
            let c = PAD_CENTS[k] * spread + rng.range(-1.5, 1.5);
            v.ratio[k] = (c / 1200.0).exp2();
        }
        v.note = note as f32;
        v.vel = vel;
        v.age = counter;
        v.env.set(attack, 1.6, 0.85, release, sr);
        v.env.gate_on();
    }

    pub fn release_all(&mut self) {
        for v in self.v.iter_mut() {
            if v.env.active() && !v.env.releasing() {
                v.env.gate_off();
            }
        }
    }

    /// Let every voice go within ~60 ms (clears the way for a final hit or a new song).
    pub fn release_fast(&mut self) {
        let sr = self.sr;
        for v in self.v.iter_mut() {
            if v.env.active() {
                v.env.set_release(0.06, sr);
                v.env.gate_off();
            }
        }
    }

    #[inline]
    pub fn process(&mut self, cutoff: f32, reso: f32, shimmer: f32) -> f32 {
        let sr = self.sr;
        self.pwm = (self.pwm + 0.21 / sr).fract();
        let pw = 0.5 + 0.32 * (self.pwm * TAU).sin();
        let mut out = 0.0;
        for v in self.v.iter_mut() {
            if !v.env.active() {
                continue;
            }
            let e = v.env.tick();
            let dt = mtof(v.note) / sr;
            let mut s = 0.0;
            for k in 0..5 {
                s += v.osc[k].saw(dt * v.ratio[k]);
            }
            s *= 0.2;
            s += 0.28 * v.pw.pulse(dt, pw);
            if shimmer > 0.001 {
                s += 0.22 * shimmer * v.hi.saw(dt * 2.003);
            }
            let key = ((v.note - 60.0) / 12.0 * 0.35).exp2();
            let cut = cutoff * key * (0.5 + 0.5 * e);
            out += v.filt.process(s, cut, reso, sr) * e * v.vel;
        }
        out
    }
}

// ================================================================ ARP
// Plucky resonant poly-synth for sequenced arpeggios.

struct ArpVoice {
    o1: Osc,
    o2: Osc,
    sub: Osc,
    filt: Ladder,
    env: Adsr,
    fenv: Decay,
    note: f32,
    vel: f32,
    gate: u32,
    pan: f32,
}

pub struct Arp {
    v: Vec<ArpVoice>,
    next: usize,
    sr: f32,
}

impl Arp {
    pub fn new(sr: f32) -> Self {
        let v = (0..8)
            .map(|i| ArpVoice {
                o1: Osc::with_phase(i as f32 * 0.31),
                o2: Osc::with_phase(i as f32 * 0.57),
                sub: Osc::default(),
                filt: Ladder::default(),
                env: Adsr::new(0.002, 0.3, 0.3, 0.22, sr),
                fenv: Decay::default(),
                note: 60.0,
                vel: 0.0,
                gate: 0,
                pan: 0.0,
            })
            .collect();
        Arp { v, next: 0, sr }
    }

    pub fn note_on(&mut self, note: i32, vel: f32, gate: u32, pan: f32, decay: f32) {
        let sr = self.sr;
        let v = &mut self.v[self.next];
        self.next = (self.next + 1) % 8;
        v.note = note as f32;
        v.vel = vel;
        v.gate = gate.max(1);
        v.pan = pan;
        v.env.set(0.003, decay, 0.28, 0.25, sr);
        v.env.gate_on();
        v.fenv.trigger(1.0, decay * 0.45, sr);
    }

    #[inline]
    pub fn process(&mut self, cutoff: f32, env_oct: f32, reso: f32) -> (f32, f32) {
        let sr = self.sr;
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.v.iter_mut() {
            if !v.env.active() {
                continue;
            }
            if v.gate > 0 {
                v.gate -= 1;
                if v.gate == 0 {
                    v.env.gate_off();
                }
            }
            let e = v.env.tick();
            let fe = v.fenv.tick();
            let dt = mtof(v.note) / sr;
            let s = 0.5 * v.o1.saw(dt * 1.0035) + 0.5 * v.o2.saw(dt * 0.9965) + 0.32 * v.sub.pulse(dt * 0.5, 0.5);
            let key = ((v.note - 60.0) / 12.0 * 0.5).exp2();
            let cut = cutoff * key * (env_oct * fe).exp2();
            let y = v.filt.process(s, cut, reso, sr) * e * v.vel;
            let (a, b) = pan(y, v.pan);
            l += a;
            r += b;
        }
        (l, r)
    }
}

// ================================================================ BASS
// Sequencer bass: saw + pulse through the ladder, clean sine sub underneath.

struct BassVoice {
    saw: Osc,
    sq: Osc,
    sub: Osc,
    filt: Ladder,
    env: Adsr,
    fenv: Decay,
    note: f32,
    vel: f32,
    gate: u32,
}

pub struct Bass {
    v: Vec<BassVoice>,
    next: usize,
    sr: f32,
}

impl Bass {
    pub fn new(sr: f32) -> Self {
        let v = (0..3)
            .map(|_| BassVoice {
                saw: Osc::default(),
                sq: Osc::with_phase(0.25),
                sub: Osc::default(),
                filt: Ladder::default(),
                env: Adsr::new(0.002, 0.22, 0.55, 0.06, sr),
                fenv: Decay::default(),
                note: 36.0,
                vel: 0.0,
                gate: 0,
            })
            .collect();
        Bass { v, next: 0, sr }
    }

    pub fn note_on(&mut self, note: i32, vel: f32, gate: u32) {
        // release whatever is still held: the bass is monophonic in spirit
        for v in self.v.iter_mut() {
            v.env.gate_off();
            v.gate = 0;
        }
        let sr = self.sr;
        let n = self.v.len();
        let v = &mut self.v[self.next];
        self.next = (self.next + 1) % n;
        v.note = note as f32;
        v.vel = vel;
        v.gate = gate.max(1);
        v.env.gate_on();
        v.fenv.trigger(1.0, 0.085, sr);
    }

    #[inline]
    pub fn process(&mut self, cutoff: f32, env_oct: f32, reso: f32) -> f32 {
        let sr = self.sr;
        let mut out = 0.0;
        for v in self.v.iter_mut() {
            if !v.env.active() {
                continue;
            }
            if v.gate > 0 {
                v.gate -= 1;
                if v.gate == 0 {
                    v.env.gate_off();
                }
            }
            let e = v.env.tick();
            let fe = v.fenv.tick();
            let dt = mtof(v.note) / sr;
            let s = 0.6 * v.saw.saw(dt) + 0.4 * v.sq.pulse(dt, 0.42);
            let cut = cutoff * (env_oct * fe * v.vel).exp2();
            let y = tanh_fast(1.6 * v.filt.process(s, cut, reso, sr));
            let sub = v.sub.sine(dt * 0.5) * 0.55;
            out += (0.75 * y + sub) * e * v.vel;
        }
        out
    }
}

// ================================================================ DRONE
// Ominous low pedal on the key root + a breathing band of filtered noise ("air").

pub struct Drone {
    o1: Osc,
    o2: Osc,
    o3: Osc,
    sub: Osc,
    filt: Ladder,
    note: f32,
    target: f32,
    lfo: f32,
    lfo2: f32,
    air: [Noise; 2],
    air_f: [Svf; 2],
    air_ph: f32,
    tick: u32,
    sr: f32,
}

impl Drone {
    pub fn new(sr: f32, note: f32) -> Self {
        Drone {
            o1: Osc::default(),
            o2: Osc::with_phase(0.4),
            o3: Osc::with_phase(0.7),
            sub: Osc::default(),
            filt: Ladder::default(),
            note,
            target: note,
            lfo: 0.0,
            lfo2: 0.3,
            air: [Noise::new(0x1234), Noise::new(0x9876)],
            air_f: [Svf::new(800.0, 2.0, sr), Svf::new(900.0, 2.0, sr)],
            air_ph: 0.0,
            tick: 0,
            sr,
        }
    }
    pub fn set_note(&mut self, n: i32) {
        self.target = n as f32;
    }
    /// Move to a note immediately (no glide).
    pub fn jump(&mut self, n: i32) {
        self.target = n as f32;
        self.note = n as f32;
    }
    #[inline]
    pub fn process(&mut self, bright: f32, air_amt: f32) -> (f32, f32, f32) {
        let sr = self.sr;
        self.note += (self.target - self.note) * (1.0 / (2.2 * sr));
        self.lfo = (self.lfo + 0.047 / sr).fract();
        self.lfo2 = (self.lfo2 + 0.083 / sr).fract();
        self.air_ph = (self.air_ph + 0.031 / sr).fract();
        let dt = mtof(self.note) / sr;
        let s = 0.4 * self.o1.saw(dt) + 0.4 * self.o2.saw(dt * 1.0045) + 0.25 * self.o3.saw(dt * 1.996);
        let cut = (85.0 + 170.0 * (0.5 + 0.5 * (self.lfo * TAU).sin())) * bright;
        let y = self.filt.process(s, cut, 1.7, sr);
        let sub = 0.5 * self.sub.sine(dt);
        let breath = 0.78 + 0.22 * (self.lfo2 * TAU).sin();
        let mono = (y + sub) * breath;

        if self.tick % 64 == 0 {
            let a = (self.air_ph * TAU).sin();
            let b = ((self.air_ph + 0.37) * TAU).sin();
            self.air_f[0].set(1300.0 * (1.2 * a).exp2(), 2.2, sr);
            self.air_f[1].set(1300.0 * (1.2 * b).exp2(), 2.2, sr);
        }
        self.tick = self.tick.wrapping_add(1);
        let (al, ar) = if air_amt > 0.001 {
            let swell = 0.55 + 0.45 * ((self.air_ph * 2.0 + 0.1) * TAU).sin();
            let nl = self.air_f[0].process(self.air[0].next()).1;
            let nr = self.air_f[1].process(self.air[1].next()).1;
            (nl * air_amt * swell, nr * air_amt * swell)
        } else {
            (0.0, 0.0)
        };
        (mono, al, ar)
    }
}

// ================================================================ LEAD
// Monophonic lead with glide, delayed vibrato and PWM.

pub struct Lead {
    saw: Osc,
    saw2: Osc,
    pw: Osc,
    filt: Ladder,
    env: Adsr,
    fenv: Decay,
    note: f32,
    target: f32,
    vib_ph: f32,
    vib_amt: f32,
    pwm: f32,
    gate: u32,
    vel: f32,
    sr: f32,
}

impl Lead {
    pub fn new(sr: f32) -> Self {
        Lead {
            saw: Osc::default(),
            saw2: Osc::with_phase(0.5),
            pw: Osc::default(),
            filt: Ladder::default(),
            env: Adsr::new(0.025, 0.6, 0.72, 0.45, sr),
            fenv: Decay::default(),
            note: 69.0,
            target: 69.0,
            vib_ph: 0.0,
            vib_amt: 0.0,
            pwm: 0.0,
            gate: 0,
            vel: 0.0,
            sr,
        }
    }
    pub fn note_on(&mut self, midi: i32, vel: f32, gate: u32) {
        let legato = self.env.active() && !self.env.releasing();
        self.target = midi as f32;
        if !legato {
            self.note = self.target;
            self.fenv.trigger(1.0, 0.35, self.sr);
        }
        self.vib_amt = 0.0;
        self.vel = vel;
        self.gate = gate.max(1);
        self.env.gate_on();
    }
    /// Let go of a held note (used for live playing; sequenced notes use their gate).
    pub fn release(&mut self) {
        self.gate = 0;
        self.env.gate_off();
    }
    /// The note currently held down, if any.
    pub fn held(&self) -> Option<i32> {
        if self.env.active() && !self.env.releasing() {
            Some(self.target.round() as i32)
        } else {
            None
        }
    }
    #[inline]
    pub fn process(&mut self, cutoff: f32, reso: f32) -> f32 {
        if !self.env.active() {
            return 0.0;
        }
        let sr = self.sr;
        if self.gate > 0 {
            self.gate -= 1;
            if self.gate == 0 {
                self.env.gate_off();
            }
        }
        self.note += (self.target - self.note) * (1.0 / (0.045 * sr));
        self.vib_amt += (1.0 - self.vib_amt) * (1.0 / (0.9 * sr));
        self.vib_ph = (self.vib_ph + 5.3 / sr).fract();
        self.pwm = (self.pwm + 0.17 / sr).fract();
        let vib = (self.vib_ph * TAU).sin() * 0.2 * self.vib_amt * self.vib_amt;
        let dt = mtof(self.note + vib) / sr;
        let s = 0.42 * self.saw.saw(dt)
            + 0.3 * self.saw2.saw(dt * 1.004)
            + 0.32 * self.pw.pulse(dt, 0.5 + 0.25 * (self.pwm * TAU).sin());
        let e = self.env.tick();
        let fe = self.fenv.tick();
        let cut = cutoff * (1.3 * fe).exp2();
        self.filt.process(s, cut, reso, sr) * e * self.vel
    }
}

// ================================================================ KEYS
// DX-era FM electric piano: a warm 1:1 body pair plus a short 14:1 "tine" for the attack,
// pitch-dependent decay, and a suitcase-style stereo tremolo.

#[derive(Clone, Copy, Default)]
struct KeyVoice {
    on: bool,
    f: f32,
    ph_c: f32,
    ph_m: f32,
    ph_t: f32,
    ph_tm: f32,
    t: f32,
    gate: u32,
    vel: f32,
    decay: f32,
    rel: f32,
    att: f32,
}

pub struct Keys {
    v: Vec<KeyVoice>,
    next: usize,
    trem: f32,
    sr: f32,
}

impl Keys {
    pub fn new(sr: f32) -> Self {
        Keys {
            v: vec![KeyVoice::default(); 16],
            next: 0,
            trem: 0.0,
            sr,
        }
    }

    pub fn note_on(&mut self, midi: i32, vel: f32, gate: u32) {
        // reuse a free voice, else the quietest one
        let mut idx = self.next;
        let mut best = f32::MAX;
        for (i, v) in self.v.iter().enumerate() {
            let lvl = if v.on {
                v.vel * (-v.t / v.decay.max(0.01)).exp() * v.rel
            } else {
                -1.0
            };
            if lvl < best {
                best = lvl;
                idx = i;
            }
        }
        self.next = (idx + 1) % self.v.len();
        let decay = 2.4 * (-(midi as f32 - 60.0) / 24.0).exp2();
        self.v[idx] = KeyVoice {
            on: true,
            f: mtof(midi as f32),
            gate: gate.max(1),
            vel: vel.clamp(0.0, 1.0),
            decay,
            rel: 1.0,
            ..Default::default()
        };
    }

    #[inline]
    pub fn process(&mut self) -> (f32, f32) {
        let sr = self.sr;
        let dt = 1.0 / sr;
        self.trem = (self.trem + 4.3 / sr).fract();
        let p = 0.38 * (self.trem * TAU).sin();
        let rel_c = (-1.0 / (0.16 * sr)).exp();
        let mut mono = 0.0;
        for v in self.v.iter_mut() {
            if !v.on {
                continue;
            }
            if v.gate > 0 {
                v.gate -= 1;
            } else {
                v.rel *= rel_c;
            }
            v.att += (1.0 - v.att) * 0.08;
            let t = v.t;
            let env = (-t / v.decay).exp() * v.rel * v.att;
            if env < 1e-4 && (v.gate == 0 || t > 8.0) {
                v.on = false;
                continue;
            }
            let body_i = (0.9 + 1.5 * v.vel) * (-t / 0.45).exp() + 0.22;
            let body = (v.ph_c * TAU + body_i * (v.ph_m * TAU).sin()).sin();
            let tine_i = 2.2 * v.vel * (-t / 0.025).exp();
            let tine = (v.ph_t * TAU + tine_i * (v.ph_tm * TAU).sin()).sin() * (-t / 0.22).exp();
            let inc = v.f * dt;
            v.ph_c = (v.ph_c + inc).fract();
            v.ph_m = (v.ph_m + inc * 1.0005).fract();
            v.ph_t = (v.ph_t + inc).fract();
            v.ph_tm = (v.ph_tm + inc * 14.0).fract();
            v.t += dt;
            mono += (0.78 * body + 0.32 * tine) * env * (0.35 + 0.65 * v.vel);
        }
        pan(mono, p)
    }
}

// ================================================================ BELLS
// 2-operator FM bells / glassy keys (very DX-era).

struct BellVoice {
    car: f32,
    modp: f32,
    f: f32,
    ratio: f32,
    env: Decay,
    ienv: Decay,
    att: f32,
    pan: f32,
}

pub struct Bells {
    v: Vec<BellVoice>,
    next: usize,
    sr: f32,
}

impl Bells {
    pub fn new(sr: f32) -> Self {
        let v = (0..10)
            .map(|_| BellVoice {
                car: 0.0,
                modp: 0.0,
                f: 440.0,
                ratio: 3.5,
                env: Decay::default(),
                ienv: Decay::default(),
                att: 0.0,
                pan: 0.0,
            })
            .collect();
        Bells { v, next: 0, sr }
    }
    pub fn note_on(&mut self, midi: i32, vel: f32, pan: f32, glassy: bool) {
        let sr = self.sr;
        let n = self.v.len();
        let v = &mut self.v[self.next];
        self.next = (self.next + 1) % n;
        v.f = mtof(midi as f32);
        v.ratio = if glassy { 3.5 } else { 2.0 };
        v.env.trigger(vel, if glassy { 1.6 } else { 0.9 }, sr);
        v.ienv.trigger(if glassy { 2.6 } else { 1.8 }, 0.16, sr);
        v.att = 0.0;
        v.pan = pan;
        v.car = 0.0;
        v.modp = 0.0;
    }
    #[inline]
    pub fn process(&mut self) -> (f32, f32) {
        let sr = self.sr;
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.v.iter_mut() {
            if v.env.v < 1e-4 {
                continue;
            }
            v.att += (1.0 - v.att) * 0.02;
            let e = v.env.tick();
            let idx = v.ienv.tick() + 0.25;
            let m = (v.modp * TAU).sin();
            let y = (v.car * TAU + idx * m).sin() * e * v.att;
            v.car = (v.car + v.f / sr).fract();
            v.modp = (v.modp + v.f * v.ratio / sr).fract();
            let (a, b) = pan(y, v.pan);
            l += a;
            r += b;
        }
        (l, r)
    }
}

// ================================================================ DRUMS
// 80s drum machine, fully synthesized. Snare & toms feed a gated reverb.
// Kicks and toms use a small voice pool so a retrigger never cuts a tail (no clicks).

#[derive(Clone, Copy, PartialEq)]
enum Thud {
    Kick,
    Heart,
    Tom,
}

#[derive(Clone, Copy)]
struct ThumpVoice {
    kind: Thud,
    ph: f32,
    t: f32,
    on: bool,
    vel: f32,
    f0: f32,
    lp: OnePole,
}

impl ThumpVoice {
    fn level(&self) -> f32 {
        if !self.on {
            return 0.0;
        }
        let tau = match self.kind {
            Thud::Kick => 0.3,
            Thud::Heart => 0.22,
            Thud::Tom => 0.32,
        };
        self.vel * (-self.t / tau).exp()
    }
}

pub struct Drums {
    sr: f32,
    noise: Noise,
    thumps: [ThumpVoice; 6],
    s_t: f32,
    s_on: bool,
    s_vel: f32,
    s_ph1: f32,
    s_ph2: f32,
    s_bp: Svf,
    s_hp: OnePole,
    h_ph: [f32; 6],
    h_env: Decay,
    h_bp: Svf,
    h_hp: Svf,
}

const HAT_F: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

impl Drums {
    pub fn new(sr: f32) -> Self {
        Drums {
            sr,
            noise: Noise::new(0xD12),
            thumps: [ThumpVoice {
                kind: Thud::Kick,
                ph: 0.0,
                t: 0.0,
                on: false,
                vel: 0.0,
                f0: 100.0,
                lp: OnePole::default(),
            }; 6],
            s_t: 0.0,
            s_on: false,
            s_vel: 0.0,
            s_ph1: 0.0,
            s_ph2: 0.0,
            s_bp: Svf::new(1900.0, 0.8, sr),
            s_hp: OnePole::default(),
            h_ph: [0.0; 6],
            h_env: Decay::default(),
            h_bp: Svf::new(10000.0, 1.1, sr),
            h_hp: Svf::new(7200.0, 0.7, sr),
        }
    }
    fn thump(&mut self, kind: Thud, vel: f32, f0: f32) {
        let mut best = 0;
        let mut lvl = f32::MAX;
        for (i, v) in self.thumps.iter().enumerate() {
            let l = v.level();
            if l < lvl {
                lvl = l;
                best = i;
            }
        }
        self.thumps[best] = ThumpVoice {
            kind,
            ph: 0.0,
            t: 0.0,
            on: true,
            vel,
            f0,
            lp: OnePole::default(),
        };
    }
    pub fn kick(&mut self, vel: f32, heart: bool) {
        self.thump(if heart { Thud::Heart } else { Thud::Kick }, vel, 0.0);
    }
    pub fn snare(&mut self, vel: f32) {
        self.s_on = true;
        self.s_t = 0.0;
        self.s_vel = vel;
    }
    pub fn hat(&mut self, vel: f32, open: bool) {
        self.h_env.trigger(vel, if open { 0.24 } else { 0.032 }, self.sr);
    }
    pub fn tom(&mut self, vel: f32, f0: f32) {
        self.thump(Thud::Tom, vel, f0);
    }

    /// Returns (left, right, gated-reverb send).
    #[inline]
    pub fn process(&mut self) -> (f32, f32, f32) {
        let sr = self.sr;
        let dt = 1.0 / sr;
        let mut mono = 0.0;
        let mut send = 0.0;
        let (mut hl, mut hr) = (0.0, 0.0);

        for k in 0..self.thumps.len() {
            if !self.thumps[k].on {
                continue;
            }
            let n = self.noise.next();
            let v = &mut self.thumps[k];
            let t = v.t;
            match v.kind {
                Thud::Kick | Thud::Heart => {
                    let heart = v.kind == Thud::Heart;
                    let (f, amp) = if heart {
                        (38.0 + 55.0 * (-t / 0.045).exp(), (-t / 0.22).exp())
                    } else {
                        (46.0 + 120.0 * (-t / 0.026).exp(), (-t / 0.3).exp())
                    };
                    v.ph = (v.ph + f * dt).fract();
                    let mut y = (v.ph * TAU).sin() * amp;
                    if heart {
                        y = v.lp.lp(y, onepole_a(160.0, sr));
                    } else {
                        y += n * (-t / 0.0015).exp() * 0.15;
                    }
                    mono += tanh_fast(1.6 * y) * v.vel * 0.55;
                }
                Thud::Tom => {
                    let f = v.f0 * (1.0 + 0.55 * (-t / 0.06).exp());
                    v.ph = (v.ph + f * dt).fract();
                    let y = ((v.ph * TAU).sin() * (-t / 0.32).exp() + n * 0.08 * (-t / 0.03).exp()) * v.vel;
                    mono += y * 0.8;
                    send += y * 0.9;
                }
            }
            v.t += dt;
            if v.t > 1.8 {
                v.on = false;
            }
        }

        if self.s_on {
            let t = self.s_t;
            let f1 = 182.0 * (1.0 + 0.45 * (-t / 0.012).exp());
            self.s_ph1 = (self.s_ph1 + f1 * dt).fract();
            self.s_ph2 = (self.s_ph2 + 331.0 * dt).fract();
            let tone = (0.6 * (self.s_ph1 * TAU).sin() + 0.4 * (self.s_ph2 * TAU).sin()) * (-t / 0.055).exp();
            let n = self.noise.next();
            let band = self.s_bp.process(n).1;
            let bright = self.s_hp.hp(n, onepole_a(4000.0, sr));
            let noise = (band * 1.1 + bright * 0.7) * (-t / 0.15).exp();
            let y = (tone * 0.55 + noise * 0.8) * self.s_vel;
            mono += y;
            send += y;
            self.s_t += dt;
            if self.s_t > 1.0 {
                self.s_on = false;
            }
        }

        if self.h_env.v > 1e-4 {
            let mut m = 0.0;
            for i in 0..6 {
                self.h_ph[i] = (self.h_ph[i] + HAT_F[i] * dt).fract();
                m += if self.h_ph[i] < 0.5 { 1.0 } else { -1.0 };
            }
            m = m * 0.16 + self.noise.next() * 0.35;
            let b = self.h_bp.process(m).1;
            let h = self.h_hp.process(b).2;
            let y = h * self.h_env.tick() * 4.5;
            let (a, c) = pan(y, 0.3);
            hl = a;
            hr = c;
        }

        let c = mono * std::f32::consts::FRAC_1_SQRT_2;
        (c + hl, c + hr, send)
    }
}

// ================================================================ IMPACTS
// Presentation one-shots: BOOM (sub drop + rumble), crash cymbal, noise riser.

#[derive(Clone, Copy)]
struct BoomVoice {
    t: f32,
    on: bool,
    ph: f32,
    lp: Svf,
}

pub struct Impacts {
    sr: f32,
    noise: [Noise; 2],
    booms: [BoomVoice; 2],
    next_boom: usize,
    c_env: Decay,
    c_hp: [Svf; 2],
    r_total: f32,
    r_left: f32,
    r_svf: [Svf; 2],
    tick: u32,
    pub riser_done: bool,
}

impl Impacts {
    pub fn new(sr: f32) -> Self {
        let bv = BoomVoice {
            t: 0.0,
            on: false,
            ph: 0.0,
            lp: Svf::new(1500.0, 0.8, sr),
        };
        Impacts {
            sr,
            noise: [Noise::new(0xB00), Noise::new(0xB01)],
            booms: [bv; 2],
            next_boom: 0,
            c_env: Decay::default(),
            c_hp: [Svf::new(4200.0, 0.7, sr), Svf::new(4600.0, 0.7, sr)],
            r_total: 1.0,
            r_left: 0.0,
            r_svf: [Svf::new(300.0, 3.0, sr), Svf::new(330.0, 3.0, sr)],
            tick: 0,
            riser_done: false,
        }
    }
    pub fn boom(&mut self) {
        // take the free (or older) voice; the other keeps ringing out
        let i = if !self.booms[0].on {
            0
        } else if !self.booms[1].on {
            1
        } else {
            self.next_boom
        };
        self.next_boom = 1 - i;
        let sr = self.sr;
        self.booms[i] = BoomVoice {
            t: 0.0,
            on: true,
            ph: 0.0,
            lp: Svf::new(2000.0, 0.9, sr),
        };
    }
    pub fn crash(&mut self, vel: f32) {
        let v = vel.max(self.c_env.v);
        self.c_env.trigger(v, 1.25, self.sr);
    }
    pub fn riser(&mut self, samples: f32) {
        self.r_total = samples.max(1.0);
        self.r_left = self.r_total;
    }

    /// Returns (left, right, reverb send mono).
    #[inline]
    pub fn process(&mut self) -> (f32, f32, f32) {
        let sr = self.sr;
        let (mut l, mut r, mut send) = (0.0, 0.0, 0.0);
        let upd = self.tick % 32 == 0;
        self.tick = self.tick.wrapping_add(1);

        for k in 0..2 {
            if !self.booms[k].on {
                continue;
            }
            let n = self.noise[k].next();
            let b = &mut self.booms[k];
            let t = b.t;
            let f = 30.0 + 75.0 * (-t / 0.22).exp();
            b.ph = (b.ph + f / sr).fract();
            let sub = (b.ph * TAU).sin() * (-t / 2.0).exp();
            if upd {
                b.lp.set(110.0 + 1900.0 * (-t / 0.35).exp(), 0.9, sr);
            }
            let rumble = b.lp.process(n).0 * (-t / 1.1).exp();
            let y = tanh_fast(1.4 * (sub * 0.95 + rumble * 0.9));
            l += y * 0.7;
            r += y * 0.7;
            send += y * 0.5;
            b.t += 1.0 / sr;
            if b.t > 7.0 {
                b.on = false;
            }
        }

        if self.c_env.v > 1e-4 {
            let e = self.c_env.tick();
            let a = self.c_hp[0].process(self.noise[0].next()).2 * e;
            let b = self.c_hp[1].process(self.noise[1].next()).2 * e;
            l += a * 0.5;
            r += b * 0.5;
            send += (a + b) * 0.35;
        }

        if self.r_left > 0.0 {
            let p = 1.0 - self.r_left / self.r_total;
            if upd {
                let fc = 250.0 * (5.2 * p).exp2();
                self.r_svf[0].set(fc * 0.92, 2.5, sr);
                self.r_svf[1].set(fc * 1.08, 2.5, sr);
            }
            let amp = p * p * 0.55;
            let a = self.r_svf[0].process(self.noise[0].next()).1 * amp;
            let b = self.r_svf[1].process(self.noise[1].next()).1 * amp;
            l += a;
            r += b;
            send += (a + b) * 0.3;
            self.r_left -= 1.0;
            if self.r_left <= 0.0 {
                self.riser_done = true;
            }
        }
        (l, r, send)
    }
}
