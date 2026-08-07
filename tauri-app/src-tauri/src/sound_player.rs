//! Sound playback for the plugin `audio.play` capability
//!
//! Plays a WAV file once through the default output device on a host-owned
//! thread Playback is intentionally simple: parse PCM, open a one-shot cpal
//! stream, wait until the buffer drains, then drop the stream
//! Never used on the real-time audio thread

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Play WAV files on the system output device
pub struct SoundPlayer;

impl SoundPlayer {
    /// Create a shared player instance
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    /// Queue a WAV file for playback, returning once parsing succeeds
    /// Playback happens on a detached thread so the caller never blocks
    pub fn play_wav(&self, path: &str) -> micyou_plugin::PluginResult<()> {
        let (samples, sample_rate) = parse_wav(path)
            .map_err(|e| micyou_plugin::PluginError::Runtime(format!("wav parse: {e}")))?;
        if samples.is_empty() {
            return Err(micyou_plugin::PluginError::Runtime("empty wav data".into()));
        }
        if sample_rate == 0 {
            return Err(micyou_plugin::PluginError::Runtime("bad sample rate".into()));
        }
        spawn_playback(samples, sample_rate);
        Ok(())
    }
}

/// Parse a RIFF/WAVE file into mono f32 samples (multi-channel is averaged)
fn parse_wav(path: &str) -> Result<(Vec<f32>, u32), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut offset = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data: Vec<u8> = Vec::new();
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let body = offset + 8;
        if body + size > bytes.len() {
            break;
        }
        match id {
            b"fmt " if size >= 16 => {
                channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap());
                sample_rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
                bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
            }
            b"data" => {
                data = bytes[body..body + size].to_vec();
            }
            _ => {}
        }
        offset = body + size + (size & 1);
    }
    if sample_rate == 0 || channels == 0 || data.is_empty() {
        return Err("missing fmt or data chunk".into());
    }
    let bytes_per_sample = (bits / 8) as usize;
    if bytes_per_sample == 0 {
        return Err(format!("unsupported bits_per_sample {bits}"));
    }
    let frames = data.len() / (bytes_per_sample * channels as usize);
    let mut samples = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut sum = 0f64;
        for ch in 0..channels as usize {
            let idx = (frame * channels as usize + ch) * bytes_per_sample;
            let raw = &data[idx..idx + bytes_per_sample];
            let value = match bits {
                16 => i16::from_le_bytes([raw[0], raw[1]]) as f64 / 32768.0,
                32 => f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as f64,
                8 => (raw[0] as f64 - 128.0) / 128.0,
                _ => return Err(format!("unsupported bits_per_sample {bits}")),
            };
            sum += value;
        }
        samples.push((sum / channels as f64) as f32);
    }
    Ok((samples, sample_rate))
}

/// Play samples on a detached thread; the stream is kept alive until the
/// buffer drains, then dropped so the device is released
fn spawn_playback(samples: Vec<f32>, _sample_rate: u32) {
    std::thread::spawn(move || {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return;
        };
        let Ok(supported) = device.default_output_config() else {
            return;
        };
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.config();
        let pos = Arc::new(AtomicUsize::new(0));
        let err_fn = |e| log::warn!("[sound] playback stream error: {e}");

        macro_rules! build_stream {
            ($t:ty) => {{
                let p = Arc::clone(&pos);
                let s = samples.clone();
                device
                    .build_output_stream::<$t, _, _>(
                        &config,
                        move |out: &mut [$t], _| {
                            let cur = p.load(Ordering::Relaxed);
                            let n = out.len();
                            let remaining = s.len().saturating_sub(cur);
                            let copy = remaining.min(n);
                            for i in 0..copy {
                                out[i] = <$t as cpal::Sample>::from_sample(s[cur + i]);
                            }
                            for i in copy..n {
                                out[i] = <$t as cpal::Sample>::from_sample(0.0f32);
                            }
                            p.store(cur + n, Ordering::Relaxed);
                        },
                        err_fn,
                        None,
                    )
            }};
        }

        let stream = match sample_format {
            cpal::SampleFormat::F32 => build_stream!(f32),
            cpal::SampleFormat::I16 => build_stream!(i16),
            cpal::SampleFormat::U16 => build_stream!(u16),
            cpal::SampleFormat::F64 => build_stream!(f64),
            _ => return,
        };
        let Ok(stream) = stream else {
            return;
        };
        if stream.play().is_err() {
            return;
        }
        let total = samples.len();
        while pos.load(Ordering::Relaxed) < total {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(stream);
    });
}
