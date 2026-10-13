//! Reading WAV files (for voice clips), std only.
//! Handles PCM 8/16/24/32-bit and 32/64-bit float, mono or multichannel, any sample rate.

use std::sync::Arc;

pub struct Clip {
    pub samples: Vec<f32>,
    pub rate: u32,
}

pub fn read(path: &str) -> Result<Clip, String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    parse(&data).map_err(|e| format!("{path}: {e}"))
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn parse(b: &[u8]) -> Result<Clip, String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file (convert with: ffmpeg -i in.mp3 out.wav)".into());
    }
    let mut i = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32le(b, i + 4) as usize;
        let body = i + 8;
        let end = (body + len).min(b.len());
        match id {
            b"fmt " if len >= 16 => {
                let mut format = u16le(b, body);
                let ch = u16le(b, body + 2);
                let rate = u32le(b, body + 4);
                let bits = u16le(b, body + 14);
                if format == 0xFFFE && len >= 26 {
                    format = u16le(b, body + 24); // WAVE_FORMAT_EXTENSIBLE sub-format
                }
                fmt = Some((format, ch, rate, bits));
            }
            b"data" => data = Some(&b[body..end]),
            _ => {}
        }
        i = body + len + (len & 1);
    }
    let (format, ch, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    let ch = ch.max(1) as usize;
    let bps = (bits as usize).div_ceil(8);
    if bps == 0 || rate == 0 {
        return Err("bad format".into());
    }
    let frame = bps * ch;
    let read_one = |o: usize| -> Option<f32> {
        Some(match (format, bits) {
            (1, 8) => (data[o] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([data[o], data[o + 1]]) as f32 / 32768.0,
            (1, 24) => {
                let v = i32::from_le_bytes([0, data[o], data[o + 1], data[o + 2]]) >> 8;
                v as f32 / 8_388_608.0
            }
            (1, 32) => i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as f32 / 2_147_483_648.0,
            (3, 32) => f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]),
            (3, 64) => {
                let mut a = [0u8; 8];
                a.copy_from_slice(&data[o..o + 8]);
                f64::from_le_bytes(a) as f32
            }
            _ => return None,
        })
    };
    if read_one(0).is_none() && !data.is_empty() {
        return Err(format!("unsupported WAV format {format} / {bits} bit"));
    }
    let frames = data.len() / frame;
    let mut samples = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut s = 0.0;
        for c in 0..ch {
            s += read_one(f * frame + c * bps).unwrap_or(0.0);
        }
        samples.push(s / ch as f32);
    }
    Ok(Clip { samples, rate })
}

/// Resample to `to` Hz (linear; fine for radio-band voice) and normalise the peak.
pub fn prepare(c: &Clip, to: f32) -> Arc<Vec<f32>> {
    let ratio = c.rate as f64 / to as f64;
    let n = ((c.samples.len() as f64) / ratio) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let x = i as f64 * ratio;
        let j = x as usize;
        let f = (x - j as f64) as f32;
        let a = c.samples.get(j).copied().unwrap_or(0.0);
        let b = c.samples.get(j + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * f);
    }
    let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 1e-6 {
        let g = 0.9 / peak;
        for x in out.iter_mut() {
            *x *= g;
        }
    }
    Arc::new(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav16(rate: u32, ch: u16, s: &[i16]) -> Vec<u8> {
        let mut b = Vec::new();
        let data_len = (s.len() * 2) as u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&ch.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * ch as u32 * 2).to_le_bytes());
        b.extend_from_slice(&(ch * 2).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"LIST"); // an extra chunk that must be skipped
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&[1, 2, 3, 0]);
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for v in s {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn reads_stereo_16bit_and_resamples() {
        let b = wav16(16000, 2, &[16384, 0, -16384, 0, 8192, 8192]);
        let c = parse(&b).unwrap();
        assert_eq!(c.rate, 16000);
        assert_eq!(c.samples.len(), 3);
        assert!((c.samples[0] - 0.25).abs() < 1e-3);
        let p = prepare(&c, 32000.0);
        assert_eq!(p.len(), 6);
        assert!((p.iter().fold(0.0f32, |m, x| m.max(x.abs())) - 0.9).abs() < 1e-3);
        assert!(parse(b"nope").is_err());
    }
}
