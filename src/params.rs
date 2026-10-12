//! Lock-free shared state between the audio engine and the control surfaces
//! (terminal UI, web panel, offline render script).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering::Relaxed};

pub struct AF(AtomicU32);

impl AF {
    pub fn new(v: f32) -> Self {
        AF(AtomicU32::new(v.to_bits()))
    }
    #[inline]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Relaxed))
    }
    #[inline]
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Relaxed)
    }
}

pub const LAYERS: [&str; 7] = ["drone", "pad", "arp", "bass", "drums", "lead", "bells"];
pub const L_DRONE: usize = 0;
pub const L_PAD: usize = 1;
pub const L_ARP: usize = 2;
pub const L_BASS: usize = 3;
pub const L_DRUMS: usize = 4;
pub const L_LEAD: usize = 5;
pub const L_BELLS: usize = 6;

pub const TRIGGERS: [&str; 5] = ["boom", "fill", "riser", "next", "key"];
pub const T_BOOM: usize = 0;
pub const T_FILL: usize = 1;
pub const T_RISER: usize = 2;
pub const T_NEXT: usize = 3;
pub const T_KEY: usize = 4;

pub const MODE_AUTO: u8 = 0;
pub const MODE_ON: u8 = 1;
pub const MODE_OFF: u8 = 2;
pub const MODE_NAMES: [&str; 3] = ["auto", "on", "off"];

pub struct Shared {
    // ---- controls (written by UI)
    pub intensity: AF,
    pub darkness: AF,
    pub bpm: AF,
    pub space: AF,
    pub tape: AF,
    pub volume: AF,
    pub drift_min: AF,
    pub drift_max: AF,
    pub drift: AtomicBool,
    pub hold: AtomicBool,
    pub fade: AtomicBool,
    pub quitting: AtomicBool,
    pub layer_mode: [AtomicU8; 7],
    pub triggers: [AtomicU32; 5],
    // ---- status (written by engine)
    pub st_intensity: AF,
    pub st_levels: [AF; 7],
    pub st_bar: AtomicU64,
    pub st_section: AtomicU32,
    pub st_key: AtomicU32,
    pub st_mode: AtomicU32,
    pub st_chord: std::sync::Mutex<String>,
    pub st_peak: AF,
    pub st_master: AF,
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

impl Shared {
    pub fn new() -> Self {
        Shared {
            intensity: AF::new(0.3),
            darkness: AF::new(0.62),
            bpm: AF::new(92.0),
            space: AF::new(0.6),
            tape: AF::new(0.5),
            volume: AF::new(0.85),
            drift_min: AF::new(0.15),
            drift_max: AF::new(0.75),
            drift: AtomicBool::new(false),
            hold: AtomicBool::new(false),
            fade: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
            layer_mode: Default::default(),
            triggers: Default::default(),
            st_intensity: AF::new(0.0),
            st_levels: std::array::from_fn(|_| AF::new(0.0)),
            st_bar: AtomicU64::new(0),
            st_section: AtomicU32::new(0),
            st_key: AtomicU32::new(0),
            st_mode: AtomicU32::new(0),
            st_chord: std::sync::Mutex::new(String::new()),
            st_peak: AF::new(0.0),
            st_master: AF::new(0.0),
        }
    }

    pub fn param(&self, name: &str) -> Option<(&AF, f32, f32)> {
        Some(match name {
            "intensity" => (&self.intensity, 0.0, 1.0),
            "darkness" => (&self.darkness, 0.0, 1.0),
            "bpm" | "tempo" => (&self.bpm, 50.0, 160.0),
            "space" => (&self.space, 0.0, 1.0),
            "tape" => (&self.tape, 0.0, 1.0),
            "volume" => (&self.volume, 0.0, 1.0),
            "drift_min" => (&self.drift_min, 0.0, 1.0),
            "drift_max" => (&self.drift_max, 0.0, 1.0),
            _ => return None,
        })
    }

    pub fn set_param(&self, name: &str, v: f32) -> bool {
        match self.param(name) {
            Some((p, lo, hi)) if v.is_finite() => {
                p.set(v.clamp(lo, hi));
                true
            }
            _ => false,
        }
    }

    pub fn nudge(&self, name: &str, d: f32) {
        if let Some((p, lo, hi)) = self.param(name) {
            p.set((p.get() + d).clamp(lo, hi));
        }
    }

    pub fn fire(&self, name: &str) -> bool {
        match TRIGGERS.iter().position(|&t| t == name) {
            Some(i) => {
                self.triggers[i].fetch_add(1, Relaxed);
                true
            }
            None => false,
        }
    }

    fn flag(&self, name: &str) -> Option<&AtomicBool> {
        match name {
            "drift" => Some(&self.drift),
            "hold" => Some(&self.hold),
            "fade" => Some(&self.fade),
            _ => None,
        }
    }

    pub fn toggle(&self, name: &str) -> bool {
        match self.flag(name) {
            Some(f) => {
                f.fetch_xor(true, Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn set_flag(&self, name: &str, on: bool) -> bool {
        match self.flag(name) {
            Some(f) => {
                f.store(on, Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn layer_index(name: &str) -> Option<usize> {
        LAYERS.iter().position(|&l| l == name)
    }

    pub fn set_layer(&self, name: &str, mode: &str) -> bool {
        let Some(i) = Self::layer_index(name) else { return false };
        let m = match mode {
            "auto" => MODE_AUTO,
            "on" => MODE_ON,
            "off" => MODE_OFF,
            "cycle" => (self.layer_mode[i].load(Relaxed) + 1) % 3,
            _ => return false,
        };
        self.layer_mode[i].store(m, Relaxed);
        true
    }

    pub fn cycle_layer(&self, i: usize) {
        let m = (self.layer_mode[i].load(Relaxed) + 1) % 3;
        self.layer_mode[i].store(m, Relaxed);
    }

    /// Apply a "name=value" / trigger / toggle command (used by web panel & render events).
    pub fn command(&self, cmd: &str) -> bool {
        let cmd = cmd.trim();
        if let Some((k, v)) = cmd.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if Self::layer_index(k).is_some() {
                return self.set_layer(k, v);
            }
            if let Some(f) = self.flag(k) {
                let on = matches!(v, "on" | "true" | "1" | "yes");
                f.store(on, Relaxed);
                return true;
            }
            return v.parse::<f32>().map(|x| self.set_param(k, x)).unwrap_or(false);
        }
        self.fire(cmd) || self.toggle(cmd)
    }

    pub fn key_name(&self) -> String {
        let k = self.st_key.load(Relaxed) as usize % 12;
        let m = self.st_mode.load(Relaxed) as usize % crate::music::MODES.len();
        format!("{} {}", crate::music::NOTE_NAMES[k], crate::music::MODES[m].name)
    }

    pub fn chord(&self) -> String {
        self.st_chord.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn state_json(&self) -> String {
        let mut layers = String::new();
        for (i, name) in LAYERS.iter().enumerate() {
            if i > 0 {
                layers.push(',');
            }
            layers.push_str(&format!(
                "\"{}\":{{\"mode\":\"{}\",\"level\":{:.3}}}",
                name,
                MODE_NAMES[self.layer_mode[i].load(Relaxed) as usize % 3],
                self.st_levels[i].get()
            ));
        }
        format!(
            "{{\"intensity\":{:.3},\"intensity_now\":{:.3},\"darkness\":{:.3},\"bpm\":{:.1},\"space\":{:.3},\"tape\":{:.3},\"volume\":{:.3},\"drift_min\":{:.3},\"drift_max\":{:.3},\"drift\":{},\"hold\":{},\"fade\":{},\"bar\":{},\"section\":{},\"key\":\"{}\",\"chord\":\"{}\",\"peak\":{:.3},\"layers\":{{{}}}}}",
            self.intensity.get(),
            self.st_intensity.get(),
            self.darkness.get(),
            self.bpm.get(),
            self.space.get(),
            self.tape.get(),
            self.volume.get(),
            self.drift_min.get(),
            self.drift_max.get(),
            self.drift.load(Relaxed),
            self.hold.load(Relaxed),
            self.fade.load(Relaxed),
            self.st_bar.load(Relaxed),
            self.st_section.load(Relaxed),
            self.key_name(),
            self.chord(),
            self.st_peak.get(),
            layers
        )
    }
}
