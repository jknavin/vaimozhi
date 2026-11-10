//! Microphone capture (cpal) and conversion to 16 kHz mono f32 for Whisper.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub const WHISPER_RATE: u32 = 16_000;
/// Hard safety limit so a forgotten recording cannot fill memory (~30 min at 48 kHz mono f32 ≈ 345 MB).
const MAX_SECONDS: u64 = 30 * 60;

pub struct Captured {
    /// 16 kHz mono samples in [-1, 1].
    pub samples: Vec<f32>,
    pub seconds: f32,
    /// Largest absolute sample before resampling. ~0 means the OS delivered silence.
    pub peak: f32,
}

pub struct Recording {
    stop_tx: mpsc::Sender<()>,
    join: Option<JoinHandle<Result<Captured, String>>>,
    pub paused: Arc<AtomicBool>,
}

impl Recording {
    pub fn set_paused(&self, p: bool) {
        self.paused.store(p, Ordering::SeqCst);
    }

    /// Stop capturing and return the audio.
    pub fn finish(mut self) -> Result<Captured, String> {
        let _ = self.stop_tx.send(());
        match self.join.take() {
            Some(j) => j.join().map_err(|_| "audio thread panicked".to_string())?,
            None => Err("recording already finished".into()),
        }
    }
}

pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn rms_peak(data: &[f32]) -> (f32, f32) {
    if data.is_empty() {
        return (0.0, 0.0);
    }
    let mut sum = 0.0f32;
    let mut peak = 0.0f32;
    for &s in data {
        sum += s * s;
        peak = peak.max(s.abs());
    }
    ((sum / data.len() as f32).sqrt(), peak)
}

struct Shared {
    samples: Mutex<Vec<f32>>,
    level_bits: std::sync::atomic::AtomicU32,
    peak_bits: std::sync::atomic::AtomicU32,
    paused: Arc<AtomicBool>,
}

impl Shared {
    fn push(&self, interleaved: &[f32], channels: usize, max_samples: usize) {
        if self.paused.load(Ordering::Relaxed) {
            self.level_bits.store(0f32.to_bits(), Ordering::Relaxed);
            return;
        }
        let ch = channels.max(1);
        let mono: Vec<f32> = interleaved
            .chunks(ch)
            .map(|f| f.iter().sum::<f32>() / f.len() as f32)
            .collect();
        let (rms, peak) = rms_peak(&mono);
        self.level_bits.store(rms.to_bits(), Ordering::Relaxed);
        let prev_peak = f32::from_bits(self.peak_bits.load(Ordering::Relaxed));
        if peak > prev_peak {
            self.peak_bits.store(peak.to_bits(), Ordering::Relaxed);
        }
        if let Ok(mut buf) = self.samples.lock() {
            if buf.len() < max_samples {
                buf.extend_from_slice(&mono);
            }
        }
    }
}

/// Start recording. `on_tick(level 0..1, elapsed_ms)` is called about 20 times per second from the audio thread.
pub fn start(
    device_name: &str,
    on_tick: impl Fn(f32, u64) + Send + 'static,
) -> Result<Recording, String> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let paused = Arc::new(AtomicBool::new(false));
    let paused_thread = paused.clone();
    let device_name = device_name.to_string();

    let join = std::thread::Builder::new()
        .name("vaimozhi-audio".into())
        .spawn(move || capture_thread(device_name, paused_thread, stop_rx, ready_tx, on_tick))
        .map_err(|e| format!("cannot start audio thread: {e}"))?;

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(Recording { stop_tx, join: Some(join), paused }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("timed out opening the microphone".into()),
    }
}

fn capture_thread(
    device_name: String,
    paused: Arc<AtomicBool>,
    stop_rx: mpsc::Receiver<()>,
    ready_tx: mpsc::Sender<Result<(), String>>,
    on_tick: impl Fn(f32, u64),
) -> Result<Captured, String> {
    let setup = || -> Result<(cpal::Stream, Arc<Shared>, u32), String> {
        let host = cpal::default_host();
        let device = if device_name.is_empty() {
            host.default_input_device()
        } else {
            host.input_devices()
                .ok()
                .and_then(|mut it| it.find(|d| d.name().map(|n| n == device_name).unwrap_or(false)))
                .or_else(|| host.default_input_device())
        }
        .ok_or_else(|| {
            "No microphone found. Connect one and check Windows Settings > Privacy > Microphone.".to_string()
        })?;

        let supported = device
            .default_input_config()
            .map_err(|e| format!("Cannot read microphone settings ({e}). Is microphone access allowed for desktop apps?"))?;
        let rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let max_samples = (rate as u64 * MAX_SECONDS) as usize;

        let shared = Arc::new(Shared {
            samples: Mutex::new(Vec::with_capacity(rate as usize * 60)),
            level_bits: std::sync::atomic::AtomicU32::new(0),
            peak_bits: std::sync::atomic::AtomicU32::new(0),
            paused,
        });
        let err_fn = |e| eprintln!("audio stream error: {e}");

        let stream = match format {
            SampleFormat::F32 => {
                let s = shared.clone();
                device.build_input_stream(&config, move |d: &[f32], _| s.push(d, channels, max_samples), err_fn, None)
            }
            SampleFormat::I16 => {
                let s = shared.clone();
                device.build_input_stream(
                    &config,
                    move |d: &[i16], _| {
                        let f: Vec<f32> = d.iter().map(|&x| x as f32 / 32768.0).collect();
                        s.push(&f, channels, max_samples)
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::U16 => {
                let s = shared.clone();
                device.build_input_stream(
                    &config,
                    move |d: &[u16], _| {
                        let f: Vec<f32> = d.iter().map(|&x| (x as f32 - 32768.0) / 32768.0).collect();
                        s.push(&f, channels, max_samples)
                    },
                    err_fn,
                    None,
                )
            }
            other => return Err(format!("Unsupported microphone sample format: {other:?}")),
        }
        .map_err(|e| format!("Cannot open the microphone: {e}"))?;

        stream.play().map_err(|e| format!("Cannot start the microphone: {e}"))?;
        Ok((stream, shared, rate))
    };

    let (stream, shared, rate) = match setup() {
        Ok(v) => {
            let _ = ready_tx.send(Ok(()));
            v
        }
        Err(e) => {
            let _ = ready_tx.send(Err(e.clone()));
            return Err(e);
        }
    };

    loop {
        match stop_rx.recv_timeout(Duration::from_millis(50)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let level = f32::from_bits(shared.level_bits.load(Ordering::Relaxed));
                let n = shared.samples.lock().map(|b| b.len()).unwrap_or(0);
                let elapsed_ms = n as u64 * 1000 / rate as u64;
                // Gentle curve so quiet speech still moves the meter.
                on_tick((level * 6.0).sqrt().clamp(0.0, 1.0), elapsed_ms);
            }
            _ => break,
        }
    }
    drop(stream);

    let raw = shared.samples.lock().map(|b| b.clone()).unwrap_or_default();
    let peak = f32::from_bits(shared.peak_bits.load(Ordering::Relaxed));
    let samples = resample_to_16k(&raw, rate);
    Ok(Captured { seconds: samples.len() as f32 / WHISPER_RATE as f32, samples, peak })
}

/// Box-filter downsample (or linear upsample) to 16 kHz.
pub fn resample_to_16k(input: &[f32], rate: u32) -> Vec<f32> {
    if rate == WHISPER_RATE || input.is_empty() {
        return input.to_vec();
    }
    let ratio = rate as f64 / WHISPER_RATE as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    if ratio >= 1.0 {
        for i in 0..out_len {
            let start = (i as f64 * ratio) as usize;
            let end = (((i + 1) as f64 * ratio).ceil() as usize).min(input.len()).max(start + 1);
            let slice = &input[start..end];
            out.push(slice.iter().sum::<f32>() / slice.len() as f32);
        }
    } else {
        for i in 0..out_len {
            let pos = i as f64 * ratio;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = *input.get(idx + 1).unwrap_or(&a);
            out.push(a + (b - a) * frac);
        }
    }
    out
}

/// Clean 16 kHz mono audio before recognition: remove DC offset, apply an 80 Hz high-pass filter
/// (rumble, desk thumps, mains hum), then normalise the level so quiet speakers and distant
/// microphones reach Whisper at a healthy amplitude. Gain is capped so room noise is not blown up.
pub fn preprocess(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    // One-pole high-pass, cutoff ~80 Hz at 16 kHz.
    let rc = 1.0 / (2.0 * std::f32::consts::PI * 80.0);
    let dt = 1.0 / WHISPER_RATE as f32;
    let alpha = rc / (rc + dt);
    let mut out = Vec::with_capacity(samples.len());
    let (mut prev_in, mut prev_out) = (0.0f32, 0.0f32);
    for &x in samples {
        let x = x - mean;
        let y = alpha * (prev_out + x - prev_in);
        prev_in = x;
        prev_out = y;
        out.push(y);
    }
    // Normalise using the 99.5th-percentile level so a single click does not set the gain.
    let mut mags: Vec<f32> = out.iter().map(|v| v.abs()).collect();
    let idx = (((mags.len() as f32) * 0.995) as usize).min(mags.len() - 1);
    let (_, p, _) = mags.select_nth_unstable_by(idx, |a, b| a.total_cmp(b));
    let level = *p;
    if level > 1e-4 {
        let gain = (0.7 / level).clamp(0.5, 12.0);
        for v in out.iter_mut() {
            *v = (*v * gain).clamp(-1.0, 1.0);
        }
    }
    out
}

// ---------- minimal 16-bit PCM WAV I/O (used only when "keep audio for retry" is on) ----------

pub fn write_wav(path: &std::path::Path, samples: &[f32]) -> std::io::Result<()> {
    use std::io::Write;
    let data_len = (samples.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&WHISPER_RATE.to_le_bytes())?;
    f.write_all(&(WHISPER_RATE * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        f.write_all(&v.to_le_bytes())?;
    }
    f.flush()
}

pub fn read_wav(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    if rate != WHISPER_RATE || bytes[34] != 16 {
        return Err("unsupported WAV format (expected 16 kHz, 16-bit mono)".into());
    }
    Ok(bytes[44..]
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_length_and_dc() {
        let input = vec![0.5f32; 48_000];
        let out = resample_to_16k(&input, 48_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|&v| (v - 0.5).abs() < 1e-6));
        let up = resample_to_16k(&vec![0.25f32; 8_000], 8_000);
        assert_eq!(up.len(), 16_000);
    }

    #[test]
    fn wav_roundtrip() {
        let path = std::env::temp_dir().join(format!("vaimozhi-wav-{}.wav", std::process::id()));
        let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0) - 0.5).collect();
        write_wav(&path, &samples).unwrap();
        let back = read_wav(&path).unwrap();
        assert_eq!(back.len(), samples.len());
        assert!((back[100] - samples[100]).abs() < 1e-3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn preprocess_removes_dc_and_levels_quiet_audio() {
        // Quiet 440 Hz tone riding on a DC offset.
        let n = 16_000;
        let tone: Vec<f32> = (0..n)
            .map(|i| 0.05 + 0.02 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin())
            .collect();
        let out = preprocess(&tone);
        assert_eq!(out.len(), n);
        let mean = out.iter().sum::<f32>() / n as f32;
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(mean.abs() < 0.02, "dc left: {mean}");
        // 0.02 input is boosted by the 12x gain cap (to ~0.24), never beyond 1.0.
        assert!(peak > 0.2 && peak <= 1.0, "peak {peak}");
    }

    #[test]
    fn preprocess_keeps_silence_silent() {
        let out = preprocess(&vec![0.0f32; 1000]);
        assert!(out.iter().all(|v| v.abs() < 1e-6));
        assert!(preprocess(&[]).is_empty());
    }

    #[test]
    fn rms_peak_basic() {
        let (rms, peak) = rms_peak(&[0.5, -0.5, 0.5, -0.5]);
        assert!((rms - 0.5).abs() < 1e-6 && (peak - 0.5).abs() < 1e-6);
    }
}
