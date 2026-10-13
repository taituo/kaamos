//! Sound-card output through cpal (CoreAudio on macOS, ALSA/PipeWire on Linux).

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;

use crate::engine::Engine;
use crate::params::Shared;

pub fn start(
    sh: Arc<Shared>,
    seed: u64,
    clips: Arc<Vec<crate::wav::Clip>>,
) -> Result<(cpal::Stream, u32, String), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no audio output device found".to_string())?;
    let name = device.to_string();
    let supported = device
        .default_output_config()
        .map_err(|e| format!("cannot query output config: {e}"))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate as u32;
    let channels = config.channels as usize;

    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, config, channels, rate, sh, seed, &clips),
        cpal::SampleFormat::I16 => build::<i16>(&device, config, channels, rate, sh, seed, &clips),
        cpal::SampleFormat::I32 => build::<i32>(&device, config, channels, rate, sh, seed, &clips),
        cpal::SampleFormat::U16 => build::<u16>(&device, config, channels, rate, sh, seed, &clips),
        other => Err(format!("unsupported sample format {other:?}")),
    }?;
    stream.play().map_err(|e| format!("cannot start audio: {e}"))?;
    Ok((stream, rate, name))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    rate: u32,
    sh: Arc<Shared>,
    seed: u64,
    clips: &[crate::wav::Clip],
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut engine = Engine::new(rate as f32, sh, seed);
    engine.set_voices(clips);
    let ch = channels.max(1);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(ch) {
                    let (l, r) = engine.next();
                    if ch == 1 {
                        frame[0] = T::from_sample((l + r) * 0.5);
                    } else {
                        frame[0] = T::from_sample(l);
                        frame[1] = T::from_sample(r);
                        for s in frame.iter_mut().skip(2) {
                            *s = T::from_sample(0.0f32);
                        }
                    }
                }
            },
            |err| eprintln!("kaamos: audio stream error: {err}"),
            None,
        )
        .map_err(|e| format!("cannot open audio stream: {e}"))
}
