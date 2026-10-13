//! Offline rendering to WAV, with an intensity curve and timed events.
//! Same engine as live mode, so a render is a faithful preview.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::sync::Arc;

use crate::dsp::Rng;
use crate::engine::Engine;
use crate::params::Shared;
use crate::Opts;

pub struct WavWriter {
    w: BufWriter<File>,
    frames: u32,
    rng: Rng,
}

impl WavWriter {
    pub fn create(path: &str, rate: u32) -> std::io::Result<Self> {
        let mut w = BufWriter::new(File::create(path)?);
        let ch: u16 = 2;
        let bits: u16 = 16;
        w.write_all(b"RIFF")?;
        w.write_all(&0u32.to_le_bytes())?;
        w.write_all(b"WAVEfmt ")?;
        w.write_all(&16u32.to_le_bytes())?;
        w.write_all(&1u16.to_le_bytes())?;
        w.write_all(&ch.to_le_bytes())?;
        w.write_all(&rate.to_le_bytes())?;
        w.write_all(&(rate * ch as u32 * bits as u32 / 8).to_le_bytes())?;
        w.write_all(&(ch * bits / 8).to_le_bytes())?;
        w.write_all(&bits.to_le_bytes())?;
        w.write_all(b"data")?;
        w.write_all(&0u32.to_le_bytes())?;
        Ok(WavWriter {
            w,
            frames: 0,
            rng: Rng::new(99),
        })
    }
    #[inline]
    pub fn push(&mut self, l: f32, r: f32) -> std::io::Result<()> {
        for x in [l, r] {
            let tpdf = (self.rng.f() - self.rng.f()) / 32768.0;
            let v = ((x + tpdf).clamp(-1.0, 1.0) * 32767.0).round() as i16;
            self.w.write_all(&v.to_le_bytes())?;
        }
        self.frames += 1;
        Ok(())
    }
    pub fn finish(mut self) -> std::io::Result<()> {
        let data = self.frames * 4;
        self.w.flush()?;
        let f = self.w.get_mut();
        f.seek(SeekFrom::Start(4))?;
        f.write_all(&(36 + data).to_le_bytes())?;
        f.seek(SeekFrom::Start(40))?;
        f.write_all(&data.to_le_bytes())?;
        Ok(())
    }
}

fn parse_time(s: &str) -> Option<f32> {
    // "90", "90s", "1m30", "1.5m"
    let s = s.trim();
    if let Some((m, rest)) = s.split_once('m') {
        let m: f32 = m.parse().ok()?;
        let rest = rest.trim_end_matches('s');
        let sec: f32 = if rest.is_empty() { 0.0 } else { rest.parse().ok()? };
        return Some(m * 60.0 + sec);
    }
    s.trim_end_matches('s').parse().ok()
}

/// "0:0.1, 60:0.5, 2m:1" -> sorted (time, value) points
pub fn parse_curve(s: &str) -> Result<Vec<(f32, f32)>, String> {
    let mut v = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (t, x) = part
            .split_once(':')
            .ok_or_else(|| format!("bad curve point '{part}' (want time:value)"))?;
        let t = parse_time(t).ok_or_else(|| format!("bad time in '{part}'"))?;
        let x: f32 = x.trim().parse().map_err(|_| format!("bad value in '{part}'"))?;
        v.push((t, x.clamp(0.0, 1.0)));
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(v)
}

fn eval_curve(c: &[(f32, f32)], t: f32) -> f32 {
    if c.is_empty() {
        return 0.0;
    }
    if t <= c[0].0 {
        return c[0].1;
    }
    for w in c.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
            return a.1 + (b.1 - a.1) * k;
        }
    }
    c[c.len() - 1].1
}

/// "30:boom, 45:arp=off, 1m:intensity=0.9, 2m:fade"
pub fn parse_events(s: &str) -> Result<Vec<(f32, String)>, String> {
    let mut v = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (t, c) = part
            .split_once(':')
            .ok_or_else(|| format!("bad event '{part}' (want time:command)"))?;
        let t = parse_time(t).ok_or_else(|| format!("bad time in '{part}'"))?;
        v.push((t, c.trim().to_string()));
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(v)
}

pub fn run(opts: &Opts) -> Result<(), String> {
    let path = opts.output.clone().unwrap_or_else(|| "kaamos.wav".into());
    let rate = opts.rate.unwrap_or(48000);
    let secs = opts.seconds.unwrap_or(180.0).max(1.0);
    let seed = opts.seed;

    let sh = Arc::new(Shared::new());
    opts.apply(&sh);
    let curve = match &opts.curve {
        Some(c) => parse_curve(c)?,
        None => Vec::new(),
    };
    let events = match &opts.events {
        Some(e) => parse_events(e)?,
        None => Vec::new(),
    };
    for (_, c) in &events {
        // validate on a scratch state so typos fail fast
        let scratch = Shared::new();
        if !scratch.command(c) {
            return Err(format!("unknown event command '{c}'"));
        }
    }
    if !curve.is_empty() {
        sh.intensity.set(eval_curve(&curve, 0.0));
    }

    let mut eng = Engine::new(rate as f32, sh.clone(), seed);
    let mut ex = crate::strudel::Exporter::new(&opts.export_dir);
    let mut wav = WavWriter::create(&path, rate).map_err(|e| format!("cannot write {path}: {e}"))?;
    let total = (secs * rate as f32) as u64;
    let fade_len = if opts.no_fade { 0.0 } else { (secs * 0.25).min(8.0) };
    let mut ev = 0usize;
    let mut peak = 0.0f32;
    let mut sumsq = 0.0f64;
    let mut last_pct = u64::MAX;
    eprintln!("kaamos: rendering {secs:.0}s @ {rate} Hz, seed {seed} -> {path}");

    for n in 0..total {
        let t = n as f32 / rate as f32;
        if n % rate as u64 == 0 {
            if let Some(m) = ex.flush(&sh) {
                eprintln!("  {m}");
            }
        }
        if n % 64 == 0 {
            if !curve.is_empty() {
                sh.intensity.set(eval_curve(&curve, t));
            }
            while ev < events.len() && events[ev].0 <= t {
                sh.command(&events[ev].1);
                ev += 1;
            }
        }
        let (mut l, mut r) = eng.next();
        let left = secs - t;
        if fade_len > 0.0 && left < fade_len {
            let p = (left / fade_len).clamp(0.0, 1.0);
            let g = p * p * p;
            l *= g;
            r *= g;
        }
        peak = peak.max(l.abs()).max(r.abs());
        sumsq += (l as f64 * l as f64 + r as f64 * r as f64) * 0.5;
        wav.push(l, r).map_err(|e| e.to_string())?;
        let pct = n * 10 / total;
        if pct != last_pct {
            last_pct = pct;
            eprintln!(
                "  {:>3}%  {:>5.1}s  intensity {:.2}  {}  {}",
                pct * 10,
                t,
                sh.st_intensity.get(),
                sh.key_name(),
                sh.chord()
            );
        }
    }
    wav.finish().map_err(|e| e.to_string())?;
    // captures fired in the last second still need writing (the engine queues them)
    for _ in 0..64 {
        eng.next();
    }
    if let Some(m) = ex.flush(&sh) {
        eprintln!("  {m}");
    }
    let rms = (sumsq / total as f64).sqrt() as f32;
    eprintln!(
        "kaamos: done. peak {:.1} dBFS, rms {:.1} dBFS, {} sections",
        20.0 * peak.max(1e-9).log10(),
        20.0 * rms.max(1e-9).log10(),
        sh.st_section.load(std::sync::atomic::Ordering::Relaxed)
    );
    Ok(())
}

/// Render a whole song (intro to ending) to WAV, printing its plan.
pub fn song(opts: &Opts) -> Result<(), String> {
    use crate::song::{Song, SongOpts};
    use std::sync::atomic::Ordering::Relaxed;

    let rate = opts.rate.unwrap_or(48000);
    let sh = Arc::new(Shared::new());
    opts.apply(&sh);
    let clips = opts.load_voices()?;
    let mut eng = Engine::new(rate as f32, sh.clone(), opts.seed);
    eng.set_voices(&clips);
    let bpm = opts.params.iter().find(|(k, _)| k == "bpm").map(|x| x.1);
    let song = Song::plan(&SongOpts {
        seed: opts.seed,
        darkness: sh.darkness.get(),
        bpm,
        target_secs: opts.seconds,
        voice_secs: eng.voice_secs(),
        radio: opts.radio,
        ending: opts.ending,
    });
    eprint!("{}", song.describe(clips.len()));
    let path = opts
        .output
        .clone()
        .unwrap_or_else(|| format!("kaamos-song-{}.wav", opts.seed));
    let total = song.total_secs();
    eng.start_song_now(song, opts.strudel);

    let events = match &opts.events {
        Some(e) => parse_events(e)?,
        None => Vec::new(),
    };
    for (_, c) in &events {
        if !Shared::new().command(c) {
            return Err(format!("unknown event command '{c}'"));
        }
    }
    let mut ev = 0usize;
    let mut ex = crate::strudel::Exporter::new(&opts.export_dir);
    let mut wav = WavWriter::create(&path, rate).map_err(|e| format!("cannot write {path}: {e}"))?;
    let done0 = sh.song_done.load(Relaxed);
    let limit = ((total + 60.0) * rate as f32) as u64;
    let (mut peak, mut sumsq, mut n) = (0.0f32, 0.0f64, 0u64);
    let mut last_status = String::new();
    loop {
        if n % 64 == 0 {
            let t = n as f32 / rate as f32;
            while ev < events.len() && events[ev].0 <= t {
                sh.command(&events[ev].1);
                ev += 1;
            }
        }
        let (l, r) = eng.next();
        peak = peak.max(l.abs()).max(r.abs());
        sumsq += (l as f64 * l as f64 + r as f64 * r as f64) * 0.5;
        wav.push(l, r).map_err(|e| e.to_string())?;
        n += 1;
        if n % 1024 == 0 {
            if sh.song_done.load(Relaxed) != done0 || n > limit {
                break;
            }
            let st = sh.song_status();
            let name = st.split(" · ").next().unwrap_or("").to_string();
            if name != last_status.split(" · ").next().unwrap_or("") && !name.is_empty() {
                eprintln!("  rendering {st}");
            }
            last_status = st;
        }
        if n % rate as u64 == 0 {
            if let Some(m) = ex.flush(&sh) {
                eprintln!("  {m}");
            }
        }
    }
    wav.finish().map_err(|e| e.to_string())?;
    if let Some(m) = ex.flush(&sh) {
        eprintln!("  {m}");
    }
    let rms = (sumsq / n.max(1) as f64).sqrt() as f32;
    eprintln!(
        "kaamos: {path} · {} · peak {:.1} dBFS, rms {:.1} dBFS",
        crate::song::mmss(n as f32 / rate as f32),
        20.0 * peak.max(1e-9).log10(),
        20.0 * rms.max(1e-9).log10()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_and_times() {
        let c = parse_curve("0:0, 1m:1, 30:0.5").unwrap();
        assert_eq!(c.len(), 3);
        assert!((eval_curve(&c, 45.0) - 0.75).abs() < 1e-4);
        assert_eq!(parse_time("1m30"), Some(90.0));
        let e = parse_events("10:boom, 20:arp=off").unwrap();
        assert_eq!(e[1].1, "arp=off");
    }
}
