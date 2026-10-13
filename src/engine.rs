//! The machine: clock, procedural composer, intensity-driven arrangement and the mix.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::dsp::*;
use crate::fx::*;
use crate::instruments::*;
use crate::music::*;
use crate::params::*;
use crate::radio::Radio;
use crate::song::*;

const CTL: u32 = 32; // control-rate block size

// Mix levels (tuned by measurement against rendered output).
const PAD_GAIN: f32 = 0.55;
const DRONE_GAIN: f32 = 0.42;
const AIR_GAIN: f32 = 1.0;
const ARP_GAIN: f32 = 0.52;
const BASS_GAIN: f32 = 0.42;
const LEAD_GAIN: f32 = 0.6;
const BELL_GAIN: f32 = 0.24;
const KEYS_GAIN: f32 = 0.2;
const DRUM_GAIN: f32 = 0.62;
const MASTER_GAIN: f32 = 1.35;
const RADIO_GAIN: f32 = 1.0;
/// lowest note of the arpeggio (keeps it under the lead)
const ARP_LO: i32 = 52;

/// Where the machine is: free-running, or playing a whole song.
enum SongState {
    Free,
    /// a song was asked for; fading the free music out first
    Pending(Box<Song>),
    Playing(Box<SongRun>),
    /// the song was stopped; fading out before going back to free mode
    Stopping,
    /// the song ended; a short silence before free mode fades back in
    Returning(f32),
}

struct SongRun {
    song: Song,
    idx: usize,
    entered: bool,
    elapsed: f32,
    skip: bool,
}

pub struct Engine {
    sr: f32,
    sh: Arc<Shared>,
    rng: Rng,

    // smoothed controls
    inten: f32,
    dark: f32,
    bpm: f32,
    space: f32,
    tape_amt: f32,
    vol: f32,
    gains: [f32; NL],
    fader: f32,
    fill_gain: f32,

    // clock
    step_phase: f32,
    step_inc: f32,
    step: u64,
    started: bool,
    bar: u64,
    bar_in_sec: u32,
    section: u32,

    // harmony & section plan
    key: i32,
    mode: usize,
    prog: Vec<i32>,
    bpc: u32,
    loop_bars: u32,
    sec_bars: u32,
    motif: Vec<usize>,
    keys_pat: usize,
    /// what the lead plays over the current chord (accompaniment stays clear of it)
    melody_pcs: Vec<i32>,
    // live lead from the phone pads
    play_seen: u32,
    live_timer: f32,
    auto_lead_g: f32,
    lead_hist: [i32; 2],
    arp_count: u32,
    arp_rate: u32,
    bass_pats: [usize; 3],
    bass_level: usize,
    lead_line: Vec<LeadNote>,
    chord_idx: usize,
    chord_deg: i32,
    arp_notes: Vec<i32>,
    bass_root: i32,
    drum_level: u8,
    drum_var: u8,
    fill_bar: bool,
    fill_forced: bool,
    fill_kind: u8,
    want_next: bool,
    want_key: bool,
    want_fill: bool,

    // modulation / bookkeeping
    sweep: f32,
    sweep2: f32,
    duck: f32,
    duck_c: f32,
    duck_a: f32,
    duck_rising: bool,
    drift_target: f32,
    drift_timer: f32,
    trig_seen: [u32; N_TRIG],
    extra_loops: u32,
    was_locked: bool,
    seed: u64,
    export_pending: bool,

    // song mode
    song_state: SongState,
    song_gain: f32,
    song_export: bool,
    song_export_pending: Option<(String, u32, [f32; NL], u64)>,
    sec_fresh: bool,
    seq_stopped: bool,
    stop_release: u32,
    fill_force: Option<u8>,
    snap_timer: f32,
    status_timer: u32,
    radio: Radio,
    ctl: u32,
    peak: f32,
    gate: f32,
    gate_hold: f32,

    // derived control values
    c_pad_cut: f32,
    c_pad_res: f32,
    c_shimmer: f32,
    c_arp_cut: f32,
    c_arp_env: f32,
    c_bass_cut: f32,
    c_bass_env: f32,
    c_lead_cut: f32,
    c_drone_bright: f32,
    c_air: f32,
    c_echo_t: f32,
    c_echo_fb: f32,
    c_wet: f32,
    c_duck_depth: f32,

    // instruments
    pad: Pad,
    arp: Arp,
    bass: Bass,
    drone: Drone,
    lead: Lead,
    solo: Lead,
    bells: Bells,
    keys: Keys,
    drums: Drums,
    imp: Impacts,

    // effects
    chorus: Chorus,
    lead_chorus: Chorus,
    boom_duck: f32,
    boom_duck_c: f32,
    echo: Echo,
    verb: Reverb,
    gverb: Reverb,
    tape: Tape,
    lim: Limiter,
    dc: [DcBlock; 2],
}

/// How many chord tones the pad uses: lush 9ths when calm, plain triads at the peak.
fn chord_ext(i: f32) -> usize {
    if i < 0.4 {
        5
    } else if i < 0.78 {
        4
    } else {
        3
    }
}

fn auto_gain(layer: usize, i: f32) -> f32 {
    match layer {
        L_DRONE => 1.0 - 0.55 * smoothstep(0.45, 0.9, i),
        L_PAD => 1.0 - 0.2 * smoothstep(0.7, 1.0, i),
        L_ARP => smoothstep(0.1, 0.3, i),
        L_BASS => smoothstep(0.3, 0.45, i),
        L_DRUMS => smoothstep(0.36, 0.44, i),
        L_LEAD => smoothstep(0.72, 0.84, i),
        L_KEYS => smoothstep(0.18, 0.32, i) * (1.0 - 0.6 * smoothstep(0.78, 1.0, i)),
        _ => 0.45 * (0.55 + 0.45 * smoothstep(0.0, 0.2, i)) * (1.0 - 0.75 * smoothstep(0.55, 0.85, i)),
    }
}

impl Engine {
    pub fn new(sr: f32, sh: Arc<Shared>, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let pc = *rng.pick(&[9, 2, 4, 0, 5, 7, 11, 6, 1]);
        let key = 45 + (pc - 9i32).rem_euclid(12);
        let dark = sh.darkness.get();
        let mode = pick_mode(&mut rng, dark);
        let mut e = Engine {
            sr,
            rng,
            inten: sh.intensity.get(),
            dark,
            bpm: sh.bpm.get(),
            space: sh.space.get(),
            tape_amt: sh.tape.get(),
            vol: sh.volume.get(),
            gains: [0.0; NL],
            fader: 0.0,
            fill_gain: 0.0,
            step_phase: 1.0,
            step_inc: 0.0,
            step: 0,
            started: false,
            bar: 0,
            bar_in_sec: 0,
            section: 0,
            key,
            mode,
            prog: vec![0, 5],
            bpc: 2,
            loop_bars: 4,
            sec_bars: 16,
            motif: vec![0, 1, 2, 3],
            keys_pat: 0,
            melody_pcs: Vec::new(),
            play_seen: sh.play_seq.load(Relaxed),
            live_timer: 1e9,
            auto_lead_g: 1.0,
            lead_hist: [-1, -1],
            arp_count: 0,
            arp_rate: 2,
            bass_pats: [0; 3],
            bass_level: 0,
            lead_line: Vec::new(),
            chord_idx: usize::MAX,
            chord_deg: 0,
            arp_notes: vec![57, 60, 64, 67],
            bass_root: 33,
            drum_level: 0,
            drum_var: 0,
            fill_bar: false,
            fill_forced: false,
            fill_kind: 0,
            want_next: true,
            want_key: false,
            want_fill: false,
            sweep: 0.0,
            sweep2: 0.5,
            duck: 0.0,
            duck_c: (-1.0 / (0.16 * sr)).exp(),
            duck_a: coef(0.004, sr),
            duck_rising: false,
            drift_target: 0.4,
            drift_timer: 0.0,
            trig_seen: std::array::from_fn(|i| sh.triggers[i].load(Relaxed)),
            extra_loops: 0,
            was_locked: false,
            seed,
            export_pending: false,
            song_state: SongState::Free,
            song_gain: 1.0,
            song_export: false,
            song_export_pending: None,
            sec_fresh: false,
            seq_stopped: false,
            stop_release: 0,
            fill_force: None,
            snap_timer: 0.0,
            status_timer: 0,
            radio: Radio::new(sr),
            ctl: 0,
            peak: 0.0,
            gate: 0.0,
            gate_hold: 0.0,
            c_pad_cut: 800.0,
            c_pad_res: 1.0,
            c_shimmer: 0.0,
            c_arp_cut: 600.0,
            c_arp_env: 2.0,
            c_bass_cut: 200.0,
            c_bass_env: 2.0,
            c_lead_cut: 1500.0,
            c_drone_bright: 1.0,
            c_air: 0.0,
            c_echo_t: 0.5,
            c_echo_fb: 0.4,
            c_wet: 0.6,
            c_duck_depth: 0.0,
            pad: Pad::new(sr, seed),
            arp: Arp::new(sr),
            bass: Bass::new(sr),
            drone: Drone::new(sr, (key - 12) as f32),
            lead: Lead::new(sr),
            solo: Lead::new(sr),
            bells: Bells::new(sr),
            keys: Keys::new(sr),
            drums: Drums::new(sr),
            imp: Impacts::new(sr),
            chorus: Chorus::new(sr),
            lead_chorus: Chorus::new(sr),
            boom_duck: 0.0,
            boom_duck_c: (-1.0 / (0.7 * sr)).exp(),
            echo: Echo::new(sr),
            verb: Reverb::new(sr, 1.9, 28.0, 5200.0),
            gverb: Reverb::new(sr, 0.75, 4.0, 7000.0),
            tape: Tape::new(sr, seed),
            lim: Limiter::new(sr, -1.0),
            dc: [DcBlock::default(); 2],
            sh,
        };
        e.gverb.set_t60(2.6);
        // start layers at their auto level so the first sound is already "in place"
        for l in 0..NL {
            e.gains[l] = e.layer_target(l);
        }
        e
    }

    fn layer_target(&self, l: usize) -> f32 {
        match self.sh.layer_mode[l].load(Relaxed) {
            MODE_ON => 1.0,
            MODE_OFF => 0.0,
            _ => match &self.song_state {
                SongState::Playing(r) => r.song.secs[r.idx].gains[l],
                _ => auto_gain(l, self.inten),
            },
        }
    }

    fn playing(&self) -> bool {
        matches!(self.song_state, SongState::Playing(_))
    }

    // ------------------------------------------------------------ songs

    /// Voice clips for the radio intro (resampled to the engine rate here).
    pub fn set_voices(&mut self, clips: &[crate::wav::Clip]) {
        let v = clips.iter().map(|c| crate::wav::prepare(c, self.sr)).collect();
        self.radio.set_voices(v);
    }

    pub fn voice_secs(&self) -> Vec<f32> {
        self.radio.voice_secs()
    }

    /// Start playing a song from its first bar (offline render, or after the live fade).
    /// With `export`, every section is captured as Strudel code as it starts.
    pub fn start_song_now(&mut self, song: Song, export: bool) {
        self.bpm = song.bpm;
        self.sh.bpm.set(song.bpm);
        self.step_inc = self.bpm * 4.0 / 60.0 / self.sr;
        self.song_export = export;
        self.fader = 1.0; // the song's intro does its own fading in
        self.want_next = false;
        self.want_key = false;
        self.extra_loops = 0;
        let i0 = song.secs[0].i0;
        self.sh.intensity.set(i0);
        self.inten = i0;
        self.song_state = SongState::Playing(Box::new(SongRun {
            song,
            idx: 0,
            entered: false,
            elapsed: 0.0,
            skip: false,
        }));
        for l in 0..NL {
            self.gains[l] = self.layer_target(l);
        }
    }

    fn toggle_song(&mut self) {
        match self.song_state {
            SongState::Free | SongState::Returning(_) => {
                let seed = self.rng.next() % 1_000_000;
                let song = Song::plan(&SongOpts {
                    seed,
                    darkness: self.sh.darkness.get(),
                    bpm: Some(self.sh.bpm.get()),
                    target_secs: None,
                    voice_secs: self.radio.voice_secs(),
                    radio: RadioPref::Auto,
                    ending: None,
                });
                self.song_state = SongState::Pending(Box::new(song));
            }
            SongState::Pending(_) => self.song_state = SongState::Free,
            SongState::Playing(_) | SongState::Stopping => self.song_state = SongState::Stopping,
        }
    }

    /// Clear tails so nothing from before comes back when the gain rises again.
    fn clean_slate(&mut self) {
        self.pad.release_fast();
        self.verb.clear();
        self.gverb.clear();
        self.echo.clear();
    }

    /// Called at each bar line: start a pending song once the free music has faded,
    /// or go back to free mode once a stopped song has faded.
    fn song_transitions(&mut self) {
        let silent = self.song_gain < 0.03;
        match std::mem::replace(&mut self.song_state, SongState::Free) {
            SongState::Pending(song) if silent => {
                self.clean_slate();
                self.drone.jump(song.key - 12);
                let export = self.song_export;
                self.start_song_now(*song, export);
            }
            SongState::Stopping if silent => self.end_song(false),
            other => self.song_state = other,
        }
    }

    fn end_song(&mut self, finished: bool) {
        self.seq_stopped = false;
        self.fill_force = None;
        self.extra_loops = 0;
        self.want_next = true;
        self.sh.intensity.set(0.2);
        if finished {
            self.sh.song_done.fetch_add(1, Relaxed);
            self.song_state = SongState::Returning(2.0);
        } else {
            self.clean_slate();
            self.song_state = SongState::Free;
        }
    }

    fn apply_part(&mut self, p: &Part) {
        self.key = p.key;
        self.mode = p.mode;
        self.prog = p.prog.clone();
        self.bpc = p.bpc.max(1);
        self.loop_bars = p.loop_bars();
        self.motif = p.motif.clone();
        self.bass_pats = p.bass_pats;
        self.drum_var = p.drum_var;
        self.keys_pat = p.keys_pat;
        self.lead_line = p.lead.clone();
        self.arp_count = 0;
        self.chord_idx = usize::MAX;
        self.drone.set_note(self.key - 12);
        self.sh.st_key.store(self.key.rem_euclid(12) as u32, Relaxed);
        self.sh.st_mode.store(self.mode as u32, Relaxed);
    }

    fn enter_section(&mut self, repeat: bool) {
        let SongState::Playing(run) = &self.song_state else {
            return;
        };
        let sec = run.song.secs[run.idx].clone();
        let mut part = run.song.parts[sec.part].transposed(sec.transpose);
        if sec.kind == Kind::End {
            part.prog = vec![0];
            part.bpc = 1;
            part.lead.clear();
        }
        self.apply_part(&part);
        self.sec_bars = sec.bars;
        if !repeat {
            self.section += 1;
            self.sec_fresh = true;
        }
        self.sh.st_section.store(self.section, Relaxed);
        if sec.snap {
            self.snap_timer = 0.25;
        }
        self.sh.intensity.set(sec.i0);
        self.seq_stopped = false;
    }

    fn song_bar(&mut self, hold: bool) {
        let lock = self.sh.lock.load(Relaxed);
        if self.was_locked && !lock {
            self.extra_loops = 0;
        }
        self.was_locked = lock;
        let SongState::Playing(run) = &mut self.song_state else {
            return;
        };
        let mut enter: Option<bool> = None;
        if !run.entered {
            run.entered = true;
            run.idx = 0;
            enter = Some(false);
        } else if run.skip {
            run.skip = false;
            run.idx += 1;
            enter = Some(false);
        } else if !hold && self.bar_in_sec >= run.song.secs[run.idx].bars {
            if lock || self.extra_loops > 0 {
                if !lock {
                    self.extra_loops -= 1;
                }
                enter = Some(true);
            } else {
                run.idx += 1;
                enter = Some(false);
            }
        }
        if run.idx >= run.song.secs.len() {
            self.end_song(true);
            self.free_bar(hold);
            return;
        }
        if let Some(repeat) = enter {
            self.bar_in_sec = 0;
            self.enter_section(repeat);
        }
        let SongState::Playing(run) = &self.song_state else {
            return;
        };
        let sec = run.song.secs[run.idx].clone();
        let song_seed = run.song.seed;

        let b = self.bar_in_sec.min(sec.bars.saturating_sub(1));
        if (sec.i1 - sec.i0).abs() > 1e-4 {
            let p = b as f32 / (sec.bars.max(2) - 1) as f32;
            self.sh.intensity.set(sec.i0 + (sec.i1 - sec.i0) * p);
        }
        self.drum_level = sec.drums;
        self.arp_rate = sec.arp_rate.max(1);
        self.bass_level = sec.bass_level.min(2);
        self.fill_bar = false;
        self.fill_forced = false;
        self.fill_force = None;
        // a section that is about to repeat skips its build-up (riser, fill)
        let repeating = lock || self.extra_loops > 0;
        for (eb, ev) in &sec.events {
            if *eb == self.bar_in_sec {
                self.fire_song_event(ev, repeating);
            }
        }
        if self.drum_level == 3 && !self.fill_bar && self.bar_in_sec % 4 == 3 && self.rng.chance(0.25) {
            self.fill_bar = true;
            self.fill_kind = self.rng.below(2) as u8;
        }
        if !self.seq_stopped {
            let ci = if hold && self.chord_idx != usize::MAX {
                self.chord_idx
            } else {
                ((self.bar_in_sec / self.bpc) as usize) % self.prog.len()
            };
            if ci != self.chord_idx {
                self.set_chord(ci);
            }
        }
        if self.song_export && self.sec_fresh && self.bar_in_sec == 1.min(sec.bars - 1) && sec.kind != Kind::End {
            self.sec_fresh = false;
            self.song_export_pending = Some((sec.name.clone(), sec.bars, sec.gains, song_seed));
        }
    }

    fn fire_song_event(&mut self, ev: &Ev, repeating: bool) {
        match ev {
            Ev::Radio => {
                if let SongState::Playing(r) = &self.song_state {
                    if let Some(c) = r.song.radio.clone() {
                        self.radio.start(&c);
                    }
                }
            }
            Ev::Static(secs) => self.radio.start_static(*secs),
            Ev::Riser(n) if !repeating => {
                let samples = (*n * 16) as f32 * self.sps() - 2.0;
                self.imp.riser(samples);
            }
            Ev::Fill(k) if !repeating => {
                self.fill_bar = true;
                self.fill_forced = true;
                self.fill_force = Some(*k);
            }
            Ev::Boom => {
                self.boom();
                self.imp.crash(0.6);
            }
            Ev::Crash(v) => self.imp.crash(*v),
            Ev::Hit => self.final_hit(),
            _ => {}
        }
    }

    /// The last chord: everything hits the tonic together, then the sequencers stop.
    fn final_hit(&mut self) {
        let notes = self.pad_notes(0, 1.0, &[]);
        self.pad.release_fast();
        let vel = 0.62 / (notes.len() as f32).sqrt();
        for &n in &notes {
            self.pad.note_on(n, vel, 0.004, 1.8, 1.0);
        }
        let gate = (32.0 * self.sps()) as u32;
        let (low, upper) = self.keys_voicing(0, &[]);
        self.keys.note_on(low, 0.9, gate);
        for &n in &upper {
            self.keys.note_on(n, 0.85, gate);
        }
        let root = self.bass_root_for(0);
        self.bass.note_on(root, 1.0, gate);
        self.kick(1.0);
        self.boom();
        self.imp.crash(0.8);
        self.chord_idx = 0;
        self.chord_deg = 0;
        mask_set(&self.sh.st_chord_mask, &notes);
        if let Ok(mut s) = self.sh.st_chord.try_lock() {
            *s = chord_name(self.key, self.mode, 0, 3);
        }
        self.seq_stopped = true;
        self.stop_release = 6;
    }

    #[inline]
    fn sps(&self) -> f32 {
        1.0 / self.step_inc.max(1e-9)
    }

    // ------------------------------------------------------------ control rate

    fn update_control(&mut self) {
        let rate = self.sr / CTL as f32;
        let dt = 1.0 / rate;
        let sh = self.sh.clone();

        // drift: let the machine wander between calm and intense on its own
        if sh.drift.load(Relaxed) && !self.playing() {
            self.drift_timer -= dt;
            if self.drift_timer <= 0.0 {
                let (a, b) = (sh.drift_min.get(), sh.drift_max.get());
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                self.drift_target = self.rng.range(lo, hi);
                self.drift_timer = self.rng.range(20.0, 55.0);
            }
            let cur = sh.intensity.get();
            sh.intensity.set(cur + (self.drift_target - cur) * coef(10.0, rate));
        }

        let itau = if self.playing() { 0.5 } else { 2.2 };
        self.inten += (sh.intensity.get() - self.inten) * coef(itau, rate);
        self.dark += (sh.darkness.get() - self.dark) * coef(1.0, rate);
        self.bpm += (sh.bpm.get() - self.bpm) * coef(1.2, rate);
        self.space += (sh.space.get() - self.space) * coef(0.6, rate);
        self.tape_amt += (sh.tape.get() - self.tape_amt) * coef(0.6, rate);
        self.vol += (sh.volume.get() - self.vol) * coef(0.08, rate);

        // triggers
        for t in 0..TRIGGERS.len() {
            let v = sh.triggers[t].load(Relaxed);
            if v != self.trig_seen[t] {
                // count every press (several can land in one control block), except
                // for one-shots where a burst should still fire only once
                let presses = v.wrapping_sub(self.trig_seen[t]).min(99);
                self.trig_seen[t] = v;
                let n = if t == T_LOOP { presses } else { 1 };
                for _ in 0..n {
                    self.on_trigger(t);
                }
            }
        }

        // layer gains
        let gtau = if !self.playing() {
            1.3
        } else if self.snap_timer > 0.0 {
            0.03
        } else {
            0.7
        };
        self.snap_timer -= dt;
        for l in 0..NL {
            let target = self.layer_target(l);
            self.gains[l] += (target - self.gains[l]) * coef(gtau, rate);
        }

        // song gain: dips between free mode and songs, and the song's own fade-outs
        let pos_in_bar = (self.step % 16) as f32 / 16.0;
        let bar_in_sec = self.bar_in_sec;
        let (sg_target, sg_tau) = match &mut self.song_state {
            SongState::Free => (1.0, 2.5),
            SongState::Pending(_) | SongState::Stopping => (0.0, 0.35),
            SongState::Returning(t) => {
                *t -= dt;
                (0.0, 0.35)
            }
            SongState::Playing(r) => {
                r.elapsed += dt;
                let s = &r.song.secs[r.idx];
                match s.fade_from {
                    Some(fb) if r.entered => {
                        let span = s.bars.saturating_sub(fb).max(1) as f32;
                        let p = ((bar_in_sec as f32 - fb as f32 + pos_in_bar) / span).clamp(0.0, 1.0);
                        ((1.0 - p) * (1.0 - p), 0.05)
                    }
                    _ => (1.0, 0.3),
                }
            }
        };
        self.song_gain += (sg_target - self.song_gain) * coef(sg_tau, rate);
        if let SongState::Returning(t) = self.song_state {
            if t <= 0.0 {
                self.song_state = SongState::Free;
            }
        }
        self.status_timer += 1;
        if self.status_timer as f32 >= rate * 0.25 {
            self.status_timer = 0;
            let (on, text) = match &self.song_state {
                SongState::Free => (false, String::new()),
                SongState::Pending(_) => (true, "starting".to_string()),
                SongState::Stopping => (true, "stopping".to_string()),
                SongState::Returning(_) => (false, "song over".to_string()),
                SongState::Playing(r) => {
                    let s = &r.song.secs[r.idx];
                    (
                        true,
                        format!(
                            "{} · bar {}/{} · {} / {}",
                            s.name,
                            (self.bar_in_sec + 1).min(s.bars),
                            s.bars,
                            mmss(r.elapsed),
                            mmss(r.song.total_secs())
                        ),
                    )
                }
            };
            sh.st_song_on.store(on, Relaxed);
            if let Ok(mut st) = sh.st_song.try_lock() {
                *st = text;
            }
        }
        let fill_target = if self.fill_forced { 0.85 } else { 0.0 };
        self.fill_gain += (fill_target - self.fill_gain) * coef(if self.fill_forced { 0.02 } else { 1.5 }, rate);

        // master fader (linear position, cubic taper)
        let quitting = sh.quitting.load(Relaxed);
        let (target, secs) = if quitting {
            (0.0, 0.5)
        } else if sh.fade.load(Relaxed) {
            (0.0, 10.0)
        } else {
            (1.0, 3.0)
        };
        let stepv = dt / secs;
        if self.fader < target {
            self.fader = (self.fader + stepv).min(target);
        } else {
            self.fader = (self.fader - stepv).max(target);
        }

        // derived sound parameters
        let i = self.inten;
        let d = self.dark;
        self.sweep = (self.sweep + dt / 41.0).fract();
        self.sweep2 = (self.sweep2 + dt / 27.0).fract();
        let sw = (self.sweep * TAU).sin();
        let sw2 = (self.sweep2 * TAU).sin();
        let darkmul = (-(d - 0.5) * 1.2).exp2();
        self.c_pad_cut = 470.0 * (i * 3.0 + 0.55 * sw).exp2() * darkmul;
        self.c_pad_res = 1.3 - 0.8 * i;
        self.c_shimmer = smoothstep(0.55, 0.95, i);
        self.c_arp_cut = 380.0 * (i * 2.7 + 0.85 * sw2).exp2() * darkmul;
        self.c_arp_env = 1.6 + 2.0 * i;
        self.c_bass_cut = 150.0 * (i * 1.9).exp2() * darkmul.sqrt();
        self.c_bass_env = 1.2 + 2.4 * i;
        self.c_lead_cut = 1300.0 * (i * 0.9 + 0.3 * sw).exp2() * darkmul;
        self.c_drone_bright = 0.9 + 0.9 * d;
        self.c_air = 0.07 * (0.35 + d) * (1.0 - 0.7 * i);
        self.c_echo_t = 0.75 * 60.0 / self.bpm;
        self.c_echo_fb = 0.36 + 0.22 * self.space;
        self.verb.set_t60(2.2 + 5.5 * self.space);
        self.c_wet = 0.3 + 0.7 * self.space;
        self.c_duck_depth = if self.drum_level >= 1 {
            0.5 * smoothstep(0.5, 0.9, i)
        } else {
            0.0
        };
        self.step_inc = self.bpm * 4.0 / 60.0 / self.sr;

        // status
        sh.st_intensity.set(self.inten);
        for l in 0..NL {
            sh.st_levels[l].set(self.gains[l]);
        }
        sh.st_peak.set(self.peak);
        sh.st_master.set(self.fader);
        if self.export_pending {
            // never block the audio thread: if the exporter holds the lock, retry next block
            if let Ok(mut caps) = sh.captures.try_lock() {
                caps.push(self.snapshot());
                self.export_pending = false;
            }
        }
        if let Some((label, bars, gains, seed)) = self.song_export_pending.take() {
            match sh.captures.try_lock() {
                Ok(mut caps) => {
                    let mut snap = self.snapshot_with(Some(gains));
                    snap.song = Some((label, bars));
                    snap.seed = seed;
                    caps.push(snap);
                }
                Err(_) => self.song_export_pending = Some((label, bars, gains, seed)),
            }
        }
        let per = if self.playing() { self.sec_bars } else { self.loop_bars };
        let left = self.sec_bars.saturating_sub(self.bar_in_sec) + self.extra_loops * per;
        sh.st_bars_left.store(left, Relaxed);
        sh.st_loops.store(self.extra_loops, Relaxed);
        sh.st_loop_bars.store(self.loop_bars, Relaxed);

        // live lead from the phone: play / release, and let the auto melody step aside
        let ps = sh.play_seq.load(Relaxed);
        if ps != self.play_seen {
            self.play_seen = ps;
            let n = sh.play_note.load(Relaxed);
            if n >= 0 {
                self.solo.note_on(n, 0.9, u32::MAX);
                self.push_lead(n);
                self.live_timer = 0.0;
            } else {
                self.solo.release();
            }
        }
        if self.solo.held().is_some() {
            self.live_timer = 0.0;
        } else {
            self.live_timer += dt;
        }
        let lead_target = if self.live_timer < 6.0 { 0.0 } else { 1.0 };
        let tau = if lead_target < self.auto_lead_g { 0.12 } else { 2.5 };
        self.auto_lead_g += (lead_target - self.auto_lead_g) * coef(tau, rate);

        // what is sounding, for the keyboard displays
        let lead_now = self.solo.held().or_else(|| {
            if self.gains[L_LEAD] * self.auto_lead_g > 0.05 {
                self.lead.held()
            } else {
                None
            }
        });
        sh.st_lead_note.store(lead_now.unwrap_or(-1), Relaxed);
        if self.gains[L_BASS] < 0.05 {
            sh.st_bass_note.store(-1, Relaxed);
        }
        if self.gains[L_ARP] < 0.05 {
            sh.st_arp_note.store(-1, Relaxed);
        }
        if self.gains[L_KEYS] < 0.05 {
            mask_set(&sh.st_keys_mask, &[]);
        }
        let machine = if self.lead_line.is_empty() {
            -1
        } else {
            let pos = (self.bar_in_sec % self.loop_bars.max(1)) * 16 + (self.step % 16) as u32;
            self.lead_line
                .iter()
                .find(|n| n.start >= pos)
                .unwrap_or(&self.lead_line[0])
                .midi
        };
        sh.st_machine_next.store(machine, Relaxed);
        self.peak *= 0.97;
    }

    fn on_trigger(&mut self, t: usize) {
        match t {
            T_BOOM => {
                self.boom();
                self.imp.crash(0.55);
            }
            T_FILL => self.want_fill = true,
            T_RISER => {
                let up = self.step % 16;
                let mut n = ((16 - up) % 16) as f32;
                if n < 8.0 {
                    n += 16.0;
                }
                let samples = ((1.0 - self.step_phase).max(0.0) + n) * self.sps();
                self.imp.riser(samples);
            }
            T_NEXT => {
                if let SongState::Playing(r) = &mut self.song_state {
                    r.skip = true;
                } else {
                    self.want_next = true;
                }
                self.extra_loops = 0;
            }
            T_SONG => self.toggle_song(),
            T_LOOP => self.extra_loops = (self.extra_loops + 1).min(99),
            T_EXPORT => self.export_pending = true,
            T_KEY if !self.playing() => {
                self.want_key = true;
                self.want_next = true;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ composition

    fn modulate(&mut self) {
        let shift = *self.rng.pick(&[2, 5, -3, 3, -2, 7]);
        let pc = (self.key + shift).rem_euclid(12);
        self.key = 45 + (pc - 9).rem_euclid(12);
        self.drone.set_note(self.key - 12);
    }

    fn new_section(&mut self) {
        self.section += 1;
        let i = self.inten;
        let mut key_changed = false;
        if self.want_key || (self.section > 2 && self.rng.chance(0.1)) {
            self.modulate();
            key_changed = true;
        }
        self.want_key = false;
        self.want_next = false;
        let ideal = self.dark * 6.0;
        if key_changed || (self.mode as f32 - ideal).abs() > 1.3 {
            self.mode = pick_mode(&mut self.rng, self.dark);
        }
        self.prog = pick_prog(&mut self.rng, self.mode, i);
        let mut d = self.prog.clone();
        d.sort();
        d.dedup();
        self.bpc = if d.len() <= 2 {
            if i < 0.35 {
                4
            } else {
                2
            }
        } else if i < 0.3 {
            2
        } else if i < 0.7 {
            if self.rng.chance(0.5) {
                2
            } else {
                1
            }
        } else {
            1
        };
        if self.prog.len() == 2 && self.bpc == 1 {
            self.bpc = 2;
        }
        self.loop_bars = self.prog.len() as u32 * self.bpc;
        self.sec_bars = if self.loop_bars >= 8 {
            16
        } else if i > 0.6 && self.rng.chance(0.35) {
            8
        } else {
            16
        };
        if self.section == 1 || self.rng.chance(0.55) {
            self.motif = arp_motif(&mut self.rng, 4);
        }
        self.bass_pats = [
            self.rng.below(BASS_LOW.len()),
            self.rng.below(BASS_MID.len()),
            self.rng.below(BASS_HIGH.len()),
        ];
        self.drum_var = self.rng.below(3) as u8;
        self.keys_pat = pick_keys_pattern(&mut self.rng, i);
        self.lead_line = gen_lead(&mut self.rng, self.key, self.mode, &self.prog, self.bpc);
        self.arp_count = 0;
        self.chord_idx = usize::MAX;
        self.drone.set_note(self.key - 12);
        self.sh.st_section.store(self.section, Relaxed);
        self.sh.st_key.store(self.key.rem_euclid(12) as u32, Relaxed);
        self.sh.st_mode.store(self.mode as u32, Relaxed);
    }

    fn set_chord(&mut self, ci: usize) {
        self.chord_idx = ci;
        self.chord_deg = self.prog[ci % self.prog.len()];
        let i = self.inten;
        let ext = chord_ext(i);
        self.melody_pcs = self.melody_pcs_for(ci);
        let mel = self.melody_pcs.clone();
        let notes = self.pad_notes(self.chord_deg, i, &mel);
        let low = i > 0.6;
        let upper = if low { &notes[1..] } else { &notes[..] };
        self.pad.release_all();
        let attack = 2.6 - 2.1 * i;
        let vel = 0.62 / (upper.len() as f32).sqrt();
        for &n in upper {
            self.pad.note_on(n, vel, attack, 3.6, 1.0);
        }
        if low {
            self.pad.note_on(notes[0], vel * 0.8, attack, 3.0, 0.7);
        }
        self.arp_notes = arp_tones(self.key, self.mode, self.chord_deg, 4, ARP_LO);
        self.bass_root = self.bass_root_for(self.chord_deg);
        mask_set(&self.sh.st_chord_mask, &notes);
        self.sh.st_chord_deg.store(self.chord_deg, Relaxed);
        if let Ok(mut s) = self.sh.st_chord.try_lock() {
            *s = chord_name(self.key, self.mode, self.chord_deg, ext);
        }
    }

    /// Pitch classes the lead plays over chord `ci` of the loop (empty when the lead is
    /// silent), so the accompaniment can leave room for them.
    fn melody_pcs_for(&self, ci: usize) -> Vec<i32> {
        let active = self.layer_target(L_LEAD).max(self.gains[L_LEAD]) * self.auto_lead_g > 0.1;
        if !active || self.lead_line.is_empty() {
            return Vec::new();
        }
        let span = self.bpc.max(1) * 16;
        let (a, b) = (ci as u32 * span, (ci as u32 + 1) * span);
        let mut v: Vec<i32> = self
            .lead_line
            .iter()
            .filter(|n| n.start >= a && n.start < b)
            .map(|n| n.midi.rem_euclid(12))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Pad notes for a chord: close voicing, plus a low root at higher intensity (first).
    /// Extensions that would rub against the melody are left out.
    fn pad_notes(&self, d: i32, i: f32, mel: &[i32]) -> Vec<i32> {
        let mut v = voicing_clear(self.key + 12, self.mode, d, chord_ext(i), 64.0, mel);
        if i > 0.6 {
            let mut low = deg(self.key, self.mode, d);
            while low > v[0] - 7 {
                low -= 12;
            }
            v.insert(0, low);
        }
        v
    }

    fn bass_root_for(&self, d: i32) -> i32 {
        let mut b = deg(self.key - 12, self.mode, d);
        while b > 43 {
            b -= 12;
        }
        while b < 31 {
            b += 12;
        }
        b
    }

    /// One pass of the current chord loop, exactly as the engine plays it, for export.
    fn snapshot(&self) -> crate::strudel::Snapshot {
        self.snapshot_with(None)
    }

    fn snapshot_with(&self, gains: Option<[f32; NL]>) -> crate::strudel::Snapshot {
        let i = self.inten;
        let bars = self.loop_bars.max(1);
        let bpc = self.bpc.max(1);
        let chord_at = |b: u32| self.prog[((b / bpc) as usize) % self.prog.len()];

        let mel_at = |b: u32| self.melody_pcs_for(((b / bpc) as usize) % self.prog.len());
        let pad = (0..bars).map(|b| self.pad_notes(chord_at(b), i, &mel_at(b))).collect();

        let rate = self.arp_rate.max(1);
        let mut arp = Vec::new();
        let mut count = 0usize;
        for b in 0..bars {
            let tones = arp_tones(self.key, self.mode, chord_at(b), 4, ARP_LO);
            let n = tones.len().max(1);
            for _ in 0..(16 / rate) {
                let idx = self.motif[count % self.motif.len()];
                arp.push(tones[idx % n] + 12 * (idx / n) as i32);
                count += 1;
            }
        }

        let pats: [&[BassPat]; 3] = [BASS_LOW, BASS_MID, BASS_HIGH];
        let lvl = self.bass_level.min(2);
        let pat = pats[lvl][self.bass_pats[lvl] % pats[lvl].len()];
        let mut bass = Vec::new();
        for b in 0..bars {
            let root = self.bass_root_for(chord_at(b));
            for &(st, off, len, _) in pat {
                bass.push((b * 16 + st as u32, len.round().max(1.0) as u32, root + off as i32));
            }
        }

        let lead = self.lead_line.iter().map(|n| (n.start, n.dur, n.midi)).collect();
        let bells = (0..bars)
            .map(|b| {
                let d = chord_at(b);
                (0..4)
                    .map(|j| {
                        let mut n = deg(self.key + 24, self.mode, d + 2 * j);
                        while n < 74 {
                            n += 12;
                        }
                        while n > 93 {
                            n -= 12;
                        }
                        n
                    })
                    .collect()
            })
            .collect();
        let kp = KEYS_PATTERNS[self.keys_pat % KEYS_PATTERNS.len()].1;
        let mut keys = Vec::new();
        for b in 0..bars {
            let d = chord_at(b);
            let (low, upper) = self.keys_voicing(d, &mel_at(b));
            let tones = self.keys_broken(d, low, &upper);
            for &(st, hit, len, _) in kp {
                let notes = match hit {
                    Hit::Chord => {
                        let mut v = upper.clone();
                        if st == 0 {
                            v.insert(0, low);
                        }
                        v
                    }
                    Hit::Tone(k) => vec![tones[k as usize % tones.len()]],
                };
                keys.push((b * 16 + st as u32, len.round().max(1.0) as u32, notes));
            }
        }
        let ext = chord_ext(i);
        crate::strudel::Snapshot {
            song: None,
            keys,
            seed: self.seed,
            section: self.section,
            key_name: format!(
                "{} {}",
                NOTE_NAMES[self.key.rem_euclid(12) as usize],
                MODES[self.mode].name
            ),
            chord_names: self
                .prog
                .iter()
                .map(|&d| chord_name(self.key, self.mode, d, ext))
                .collect(),
            bpm: self.bpm,
            intensity: i,
            space: self.space,
            loop_bars: bars,
            pad,
            arp,
            bass,
            lead,
            drone: self.key - 12,
            bells,
            drum_level: self.drum_level,
            drum_var: self.drum_var,
            gains: gains.unwrap_or(self.gains),
            pad_cut: self.c_pad_cut,
            arp_cut: self.c_arp_cut,
            bass_cut: self.c_bass_cut,
            lead_cut: self.c_lead_cut,
        }
    }

    fn on_bar(&mut self) {
        let hold = self.sh.hold.load(Relaxed);
        if self.started {
            self.bar += 1;
            if !hold {
                self.bar_in_sec += 1;
            }
        } else {
            self.started = true;
        }
        self.song_transitions();
        if self.playing() {
            self.song_bar(hold);
        } else {
            self.free_bar(hold);
        }
        self.sh.st_bar.store(self.bar, Relaxed);
    }

    fn free_bar(&mut self, hold: bool) {
        let i = self.inten;

        // drum / arp / bass levels with hysteresis
        let th = [0.56f32, 0.72, 0.86];
        let mut lvl = 0u8;
        for (k, &t) in th.iter().enumerate() {
            let t = if (k as u8) < self.drum_level {
                t - 0.03
            } else {
                t + 0.03
            };
            if i > t {
                lvl = k as u8 + 1;
            }
        }
        if self.sh.layer_mode[L_DRUMS].load(Relaxed) == MODE_ON && lvl == 0 && i > 0.3 {
            lvl = 1;
        }
        self.drum_level = lvl;
        self.arp_rate = match self.arp_rate {
            4 => {
                if i > 0.3 {
                    2
                } else {
                    4
                }
            }
            2 => {
                if i > 0.6 {
                    1
                } else if i < 0.22 {
                    4
                } else {
                    2
                }
            }
            _ => {
                if i < 0.52 {
                    2
                } else {
                    1
                }
            }
        };
        self.bass_level = if i < 0.58 {
            0
        } else if i < 0.82 {
            1
        } else {
            2
        };

        // pattern lock / repeats: instead of moving on, play the chord loop again
        let lock = self.sh.lock.load(Relaxed);
        if self.was_locked && !lock {
            self.extra_loops = 0; // releasing the lock also drops queued repeats
        }
        self.was_locked = lock;
        if !hold && !self.want_next && self.bar_in_sec >= self.sec_bars && (lock || self.extra_loops > 0) {
            if !lock {
                self.extra_loops -= 1;
            }
            self.sec_bars += self.loop_bars.max(1);
        }

        let new_sec = self.want_next || (!hold && self.bar_in_sec >= self.sec_bars);
        if new_sec {
            self.new_section();
            self.bar_in_sec = 0;
            if self.drum_level >= 2 && self.gains[L_DRUMS] > 0.25 {
                self.imp.crash(0.45 * self.gains[L_DRUMS]);
            }
        }

        // fills
        self.fill_forced = self.want_fill;
        let section_end = self.bar_in_sec + 1 == self.sec_bars && !lock && self.extra_loops == 0;
        let auto_fill = (self.drum_level >= 2 && section_end && self.rng.chance(0.75))
            || (self.drum_level == 3 && self.bar_in_sec % 4 == 3 && self.rng.chance(0.25));
        self.fill_bar = self.want_fill || auto_fill;
        self.fill_kind = if self.drum_level >= 2 {
            self.rng.below(2) as u8
        } else {
            0
        };
        self.want_fill = false;

        // chord
        let ci = if hold && self.chord_idx != usize::MAX {
            self.chord_idx
        } else {
            ((self.bar_in_sec / self.bpc) as usize) % self.prog.len()
        };
        if ci != self.chord_idx {
            self.set_chord(ci);
        }
    }

    fn on_step(&mut self) {
        let sib = (self.step % 16) as u32;
        if sib == 0 {
            self.on_bar();
        }
        if self.seq_stopped {
            // after the final hit: just let things ring
            if self.stop_release > 0 {
                self.stop_release -= 1;
                if self.stop_release == 0 {
                    self.pad.release_all();
                }
            }
            self.step += 1;
            return;
        }
        let sps = self.sps();
        let i = self.inten;

        // ---- arpeggio
        if self.gains[L_ARP] > 0.002 && sib % self.arp_rate == 0 {
            let n = self.arp_notes.len().max(1);
            let idx = self.motif[(self.arp_count as usize) % self.motif.len()];
            let note = self.arp_notes[idx % n] + 12 * (idx / n) as i32;
            let accent = if self.arp_count % 4 == 0 {
                1.0
            } else {
                0.72 + 0.1 * self.rng.f()
            };
            let decay = match self.arp_rate {
                1 => 0.2,
                2 => 0.28,
                _ => 0.55,
            };
            let gate = (sps * self.arp_rate as f32 * 0.6) as u32;
            let p = if self.arp_count % 2 == 0 { -0.22 } else { 0.22 };
            self.arp.note_on(note, accent, gate, p, decay);
            self.sh.st_arp_note.store(note, Relaxed);
            self.arp_count += 1;
        }

        // ---- bass
        if self.gains[L_BASS] > 0.002 {
            let pats: [&[BassPat]; 3] = [BASS_LOW, BASS_MID, BASS_HIGH];
            let pat = pats[self.bass_level][self.bass_pats[self.bass_level] % pats[self.bass_level].len()];
            for &(st, off, len, acc) in pat {
                if st as u32 == sib {
                    let n = self.bass_root + off as i32;
                    self.bass.note_on(n, 0.6 + 0.4 * acc, (len * sps) as u32);
                    self.sh.st_bass_note.store(n, Relaxed);
                }
            }
        }

        // ---- drums
        self.drum_step(sib);

        // ---- lead
        if self.gains[L_LEAD] > 0.002 && !self.lead_line.is_empty() {
            let pos = (self.bar_in_sec % self.loop_bars.max(1)) * 16 + sib;
            for k in 0..self.lead_line.len() {
                let n = self.lead_line[k];
                if n.start == pos {
                    self.lead.note_on(n.midi, 0.85, (n.dur as f32 * sps * 0.93) as u32);
                    if self.auto_lead_g > 0.5 {
                        self.push_lead(n.midi);
                    }
                }
            }
        }

        // ---- bells
        if self.gains[L_BELLS] > 0.01 && sib % 2 == 0 {
            let p = 0.04 + 0.06 * (1.0 - i);
            if self.rng.chance(p) {
                let j = self.rng.below(4) as i32;
                let mut n = deg(self.key + 24, self.mode, self.chord_deg + 2 * j);
                while n < 74 {
                    n += 12;
                }
                while n > 93 {
                    n -= 12;
                }
                let glassy = self.dark > 0.75 && self.rng.chance(0.5);
                let vel = self.rng.range(0.35, 0.8);
                let pn = self.rng.range(-0.7, 0.7);
                self.bells.note_on(n, vel, pn, glassy);
            }
        }

        // ---- keys (electric piano comping)
        if self.gains[L_KEYS] > 0.002 {
            let pat = KEYS_PATTERNS[self.keys_pat % KEYS_PATTERNS.len()].1;
            for &(st, hit, len, vel) in pat {
                if st as u32 != sib {
                    continue;
                }
                let (low, upper) = self.keys_voicing(self.chord_deg, &self.melody_pcs);
                let gate = (len * sps) as u32;
                let mut played = Vec::with_capacity(6);
                match hit {
                    Hit::Chord => {
                        for &n in &upper {
                            self.keys.note_on(n, vel, gate);
                            played.push(n);
                        }
                        if sib == 0 {
                            self.keys.note_on(low, vel * 0.8, gate);
                            played.push(low);
                        }
                    }
                    Hit::Tone(k) => {
                        let tones = self.keys_broken(self.chord_deg, low, &upper);
                        let n = tones[k as usize % tones.len()];
                        self.keys.note_on(n, vel, gate);
                        played.push(n);
                    }
                }
                mask_set(&self.sh.st_keys_mask, &played);
            }
        }

        self.step += 1;
    }

    fn push_lead(&mut self, n: i32) {
        self.lead_hist = [n, self.lead_hist[0]];
        self.sh.st_lead_prev.store(self.lead_hist[0], Relaxed);
        self.sh.st_lead_prev2.store(self.lead_hist[1], Relaxed);
    }

    /// Keys voicing: a low root under a close upper voicing (clear of the melody).
    fn keys_voicing(&self, d: i32, mel: &[i32]) -> (i32, Vec<i32>) {
        let ext = if self.inten < 0.5 { 5 } else { 4 };
        let upper = voicing_clear(self.key + 12, self.mode, d, ext, 62.0, mel);
        let mut low = deg(self.key, self.mode, d);
        while low >= upper[0] - 5 {
            low -= 12;
        }
        while low < 40 {
            low += 12;
        }
        (low, upper)
    }

    /// Broken-chord tone list: root, fifth, then the upper voicing (no doubled notes).
    fn keys_broken(&self, d: i32, low: i32, upper: &[i32]) -> Vec<i32> {
        let fifth = deg(0, self.mode, d + 4) - deg(0, self.mode, d);
        let mut t = vec![low, low + fifth];
        for &n in upper {
            if !t.contains(&n) {
                t.push(n);
            }
        }
        t
    }

    fn drum_step(&mut self, s: u32) {
        let g = self.gains[L_DRUMS];
        let forced = self.fill_forced;
        if g < 0.003 && !forced {
            return;
        }
        let fill = self.fill_bar && s >= 8;
        let lv = self.drum_level;
        let v = self.drum_var;
        let normal = g >= 0.003;

        if normal {
            match lv {
                0 => {
                    if s == 0 {
                        self.drums.kick(0.95, true);
                    } else if s == 3 {
                        self.drums.kick(0.6, true);
                    }
                }
                1 => {
                    if s == 0 || s == 8 || (v == 1 && s == 10) {
                        self.kick(0.9);
                    }
                    if s % 4 == 2 && !(fill && s >= 12) {
                        self.drums.hat(0.35, false);
                    }
                }
                2 => {
                    if s == 0 || s == 8 || (v == 1 && s == 6) || (v == 2 && s == 10) {
                        self.kick(0.92);
                    }
                    if (s == 4 || s == 12) && !fill {
                        self.drums.snare(0.85);
                    }
                    if !fill {
                        if s % 2 == 0 {
                            self.drums.hat(if s % 4 == 2 { 0.5 } else { 0.28 }, false);
                        } else if v == 2 {
                            self.drums.hat(0.15, false);
                        }
                    }
                }
                _ => {
                    if s % 4 == 0 && !(fill && s > 8) {
                        self.kick(0.95);
                    }
                    if (s == 4 || s == 12) && !fill {
                        self.drums.snare(0.9);
                    }
                    if !fill {
                        if s % 4 == 2 {
                            self.drums.hat(0.38, true);
                        } else {
                            let a = [0.42, 0.18, 0.0, 0.2][(s % 4) as usize];
                            self.drums.hat(a, false);
                        }
                    }
                }
            }
        }

        if self.fill_bar && s >= 8 {
            let toms = [195.0, 165.0, 140.0, 122.0, 106.0, 92.0];
            let kind = match self.fill_force {
                Some(k) => k,
                None if lv <= 1 || !normal => 0,
                None => self.fill_kind,
            };
            match kind {
                0 => {
                    let slots = if lv <= 1 || !normal {
                        [12, 13, 14, 15, 99, 99]
                    } else {
                        [8, 10, 12, 13, 14, 15]
                    };
                    if let Some(k) = slots.iter().position(|&x| x == s) {
                        let k = if lv <= 1 || !normal { k + 2 } else { k };
                        self.drums.tom(0.75 + 0.04 * k as f32, toms[k.min(5)]);
                    }
                }
                _ => {
                    let vel = 0.32 + (s - 8) as f32 * 0.08;
                    self.drums.snare(vel.min(1.0));
                    if s == 8 {
                        self.kick(0.9);
                    }
                }
            }
        }
    }

    fn boom(&mut self) {
        self.imp.boom();
        self.boom_duck = 1.0;
    }

    fn kick(&mut self, vel: f32) {
        self.drums.kick(vel, false);
        self.duck_rising = true;
    }

    // ------------------------------------------------------------ audio rate

    /// Render one stereo sample.
    #[inline]
    pub fn next(&mut self) -> (f32, f32) {
        if self.ctl == 0 {
            self.update_control();
        }
        self.ctl = (self.ctl + 1) % CTL;

        self.step_phase += self.step_inc;
        if self.step_phase >= 1.0 {
            self.step_phase -= 1.0;
            self.on_step();
        }
        if self.imp.riser_done {
            self.imp.riser_done = false;
            self.imp.crash(0.6);
            if self.gains[L_DRUMS] > 0.1 && self.drum_level >= 1 {
                self.kick(1.0);
            }
        }

        let g = self.gains;
        if self.duck_rising {
            self.duck += (1.08 - self.duck) * self.duck_a;
            if self.duck >= 1.0 {
                self.duck = 1.0;
                self.duck_rising = false;
            }
        } else {
            self.duck *= self.duck_c;
        }
        let pump = 1.0 - self.c_duck_depth * self.duck;

        let pad = self.pad.process(self.c_pad_cut, self.c_pad_res, self.c_shimmer) * g[L_PAD] * pump * PAD_GAIN;
        let (dr, airl, airr) = self.drone.process(self.c_drone_bright, self.c_air);
        let dr = dr * g[L_DRONE] * pump * DRONE_GAIN;
        let (airl, airr) = (airl * g[L_DRONE] * AIR_GAIN, airr * g[L_DRONE] * AIR_GAIN);
        let (al, ar) = self.arp.process(self.c_arp_cut, self.c_arp_env, 2.3);
        let ag = g[L_ARP] * (0.55 + 0.45 * pump) * ARP_GAIN * (0.6 + 0.4 * self.inten);
        let (al, ar) = (al * ag, ar * ag);
        let bass = self.bass.process(self.c_bass_cut, self.c_bass_env, 1.1)
            * g[L_BASS]
            * (1.0 - 0.5 * self.c_duck_depth * self.duck)
            * BASS_GAIN;
        let solo = self.solo.process(self.c_lead_cut.max(1500.0), 0.8) * LEAD_GAIN;
        let lead = self.lead.process(self.c_lead_cut, 0.8) * g[L_LEAD] * self.auto_lead_g * LEAD_GAIN + solo;
        let (epl, epr) = self.keys.process();
        let kg = g[L_KEYS] * KEYS_GAIN * (0.6 + 0.4 * pump);
        let (epl, epr) = (epl * kg, epr * kg);
        let (bl, br) = self.bells.process();
        let (bl, br) = (bl * g[L_BELLS] * BELL_GAIN, br * g[L_BELLS] * BELL_GAIN);
        let dg = g[L_DRUMS].max(self.fill_gain) * DRUM_GAIN;
        let (kl, kr, gsend) = self.drums.process();
        let (kl, kr, gsend) = (kl * dg, kr * dg, gsend * dg);
        let (il, ir, isend, boom) = self.imp.process();
        self.boom_duck *= self.boom_duck_c;
        let (ral, rar, rsend) = self.radio.process();
        let (il, ir, isend) = (il + ral * RADIO_GAIN, ir + rar * RADIO_GAIN, isend + rsend * RADIO_GAIN);

        // pad through the ensemble chorus
        let (cl, cr) = self.chorus.process(pad, 1.0);
        let (ll, lr) = self.lead_chorus.process(lead, 0.6);

        let dry_l = cl + dr * 0.7 + airl + al + bass * 0.7 + ll + bl + kl + il + epl;
        let dry_r = cr + dr * 0.7 + airr + ar + bass * 0.7 + lr + br + kr + ir + epr;

        // tape echo
        let echo_in = (al + ar) * 0.32 + lead * 0.5 + (bl + br) * 0.3 + (epl + epr) * 0.08;
        let (el, er) = self.echo.process(echo_in, self.c_echo_t, self.c_echo_fb);

        // hall
        let rin_l = cl * 0.5
            + al * 0.35
            + ll * 0.65
            + bl * 0.8
            + el * 0.45
            + airl * 0.6
            + isend
            + dr * 0.08
            + kl * 0.05
            + epl * 0.4;
        let rin_r = cr * 0.5
            + ar * 0.35
            + lr * 0.65
            + br * 0.8
            + er * 0.45
            + airr * 0.6
            + isend
            + dr * 0.08
            + kr * 0.05
            + epr * 0.4;
        let (rl, rr) = self.verb.process(rin_l, rin_r);
        let (rl, rr) = (rl * self.c_wet, rr * self.c_wet);

        // gated reverb (the 80s snare)
        if gsend.abs() > 0.025 {
            self.gate_hold = 0.2 * self.sr;
        }
        let gt = if self.gate_hold > 0.0 {
            self.gate_hold -= 1.0;
            1.0
        } else {
            0.0
        };
        self.gate += (gt - self.gate) * if gt > self.gate { 0.05 } else { 0.004 };
        let (gl, gr) = self.gverb.process(gsend, gsend);
        let (gl, gr) = (gl * self.gate * 1.5, gr * self.gate * 1.5);

        let l = (dry_l + el * 0.55 + rl + gl) * MASTER_GAIN;
        let r = (dry_r + er * 0.55 + rr + gr) * MASTER_GAIN;

        let bd = 1.0 - 0.35 * self.boom_duck;
        let (l, r) = self.tape.process(l * bd, r * bd, self.tape_amt);
        let (l, r) = (l + boom * MASTER_GAIN, r + boom * MASTER_GAIN);
        self.peak = self.peak.max(l.abs().max(r.abs()));
        // DC blocker before the limiter, so the limiter really is the last word on peaks
        let (l, r) = (self.dc[0].process(l), self.dc[1].process(r));
        let (l, r) = self.lim.process(l, r);
        let m = self.vol * self.vol * self.fader * self.fader * self.fader * self.song_gain;
        (l * m, r * m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 8000.0;

    fn run_bars(e: &mut Engine, bars: f32) {
        let n = (bars * 4.0 * 60.0 / 92.0 * SR) as usize;
        for _ in 0..n {
            e.next();
        }
    }

    #[test]
    fn a_live_song_plays_through_and_returns_to_free_mode() {
        let sh = Arc::new(Shared::new());
        sh.set_param("intensity", 0.7);
        let mut e = Engine::new(SR, sh.clone(), 11);
        run_bars(&mut e, 2.0);
        assert!(!sh.st_song_on.load(Relaxed));
        sh.fire("song");
        run_bars(&mut e, 0.3);
        assert!(sh.st_song_on.load(Relaxed), "song requested");
        // fades out, then starts at a bar line within a couple of bars
        run_bars(&mut e, 3.0);
        assert!(e.playing(), "song playing");
        let SongState::Playing(r) = &e.song_state else {
            unreachable!()
        };
        let total_bars = r.song.total_bars();
        let n_secs = r.song.secs.len();
        // skip one part with "next"
        let idx0 = r.idx;
        sh.fire("next");
        run_bars(&mut e, 1.2);
        let SongState::Playing(r) = &e.song_state else {
            unreachable!()
        };
        assert_eq!(r.idx, idx0 + 1, "next skips to the following part");
        // play to the end
        let done0 = sh.song_done.load(Relaxed);
        let mut bars = 0.0;
        while sh.song_done.load(Relaxed) == done0 {
            run_bars(&mut e, 1.0);
            bars += 1.0;
            assert!(bars < total_bars as f32 + 4.0, "song never ended ({n_secs} parts)");
        }
        // after a short pause free mode comes back and the music fades in again
        run_bars(&mut e, 3.0);
        assert!(matches!(e.song_state, SongState::Free));
        run_bars(&mut e, 4.0);
        assert!(e.song_gain > 0.8, "free mode fades back in ({})", e.song_gain);
        assert!(!sh.st_song_on.load(Relaxed));
    }

    #[test]
    fn stopping_a_song_returns_to_free_mode() {
        let sh = Arc::new(Shared::new());
        let mut e = Engine::new(SR, sh.clone(), 5);
        sh.fire("song");
        run_bars(&mut e, 6.0);
        assert!(e.playing());
        sh.fire("song");
        run_bars(&mut e, 3.0);
        assert!(matches!(e.song_state, SongState::Free));
        run_bars(&mut e, 6.0);
        assert!(e.song_gain > 0.8);
    }

    #[test]
    fn lock_keeps_the_pattern() {
        let sh = Arc::new(Shared::new());
        sh.set_param("intensity", 0.6);
        let mut e = Engine::new(SR, sh.clone(), 42);
        run_bars(&mut e, 0.5);
        let s0 = sh.st_section.load(Relaxed);
        sh.lock.store(true, Relaxed);
        run_bars(&mut e, 40.0);
        assert_eq!(sh.st_section.load(Relaxed), s0, "locked pattern must not change");
        sh.lock.store(false, Relaxed);
        run_bars(&mut e, 40.0);
        assert!(sh.st_section.load(Relaxed) > s0, "released pattern moves on");
    }

    #[test]
    fn repeats_extend_by_whole_loops() {
        let sh = Arc::new(Shared::new());
        sh.set_param("intensity", 0.6);
        let mut e = Engine::new(SR, sh.clone(), 7);
        run_bars(&mut e, 0.5);
        let left0 = sh.st_bars_left.load(Relaxed);
        let lb = sh.st_loop_bars.load(Relaxed);
        for _ in 0..3 {
            sh.fire("loop");
        }
        run_bars(&mut e, 0.1);
        assert_eq!(sh.st_loops.load(Relaxed), 3);
        let left = sh.st_bars_left.load(Relaxed);
        assert_eq!(left, left0 + 3 * lb);
        let s0 = sh.st_section.load(Relaxed);
        run_bars(&mut e, left as f32 - 1.0);
        assert_eq!(sh.st_section.load(Relaxed), s0, "still repeating");
        run_bars(&mut e, 1.0);
        assert_eq!(sh.st_section.load(Relaxed), s0 + 1, "moves on after the repeats");
    }
}
