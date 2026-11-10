//! Speech recognition through whisper.cpp (whisper-rs bindings).
//!
//! Written against the whisper-rs 0.14 API.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use vaimozhi_core::{hallucination, Language};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub const SAMPLE_RATE: usize = 16_000;
/// Audio is fed to Whisper in chunks of at most this many seconds, cut at the quietest nearby point.
const CHUNK_SECONDS: usize = 28;
/// Chunks whose peak amplitude is below this are treated as silence (prevents hallucinated text).
const SILENCE_PEAK: f32 = 0.008;

pub struct Engine {
    ctx: WhisperContext,
    pub model_id: String,
}

impl Engine {
    pub fn load(path: &Path, model_id: &str) -> Result<Engine, String> {
        let path_str = path.to_str().ok_or("model path is not valid UTF-8")?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .map_err(|e| format!("Could not load the model ({e}). The file may be damaged: use Verify in Models."))?;
        Ok(Engine { ctx, model_id: model_id.to_string() })
    }

    /// Transcribe 16 kHz mono audio. `progress(0..1)` is called after every chunk.
    /// Returns the raw Whisper text; hallucination filtering is applied per chunk.
    ///
    /// Accuracy measures: temperature fallback (Whisper retries a chunk at higher temperature when
    /// the first decode looks unreliable), beam search by default, a no-speech threshold, a
    /// script-matched initial prompt, and Tamil/English-only language detection in Auto mode.
    pub fn transcribe(
        &self,
        audio: &[f32],
        lang: Language,
        threads: u32,
        beam: bool,
        cancel: &AtomicBool,
        progress: impl Fn(f32),
    ) -> Result<String, String> {
        let n_threads = if threads == 0 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8) as i32
        } else {
            threads as i32
        };
        let mut state = self.ctx.create_state().map_err(|e| format!("whisper state: {e}"))?;
        let chunks = split_chunks(audio, CHUNK_SECONDS * SAMPLE_RATE);
        let total = chunks.len().max(1);
        let mut pieces: Vec<String> = Vec::new();
        let mut last_code: &'static str = "en";

        for (i, (start, end)) in chunks.iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            let chunk = &audio[*start..*end];
            let peak = chunk.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            if peak >= SILENCE_PEAK && chunk.len() > SAMPLE_RATE / 4 {
                let code: &'static str = match lang.whisper_code() {
                    Some(c) => c,
                    None => self.detect_ta_or_en(&mut state, chunk, n_threads, last_code),
                };
                last_code = code;

                let strategy = if beam {
                    SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 }
                } else {
                    SamplingStrategy::Greedy { best_of: 3 }
                };
                let mut params = FullParams::new(strategy);
                params.set_language(Some(code));
                params.set_translate(false);
                params.set_n_threads(n_threads);
                params.set_print_special(false);
                params.set_print_progress(false);
                params.set_print_realtime(false);
                params.set_print_timestamps(false);
                params.set_suppress_blank(true);
                params.set_no_speech_thold(0.6);
                params.set_logprob_thold(-1.0);
                params.set_entropy_thold(2.4);
                params.set_temperature(0.0);
                params.set_temperature_inc(0.2);
                let prompt = vaimozhi_core::prompt_for(code);
                params.set_initial_prompt(prompt);

                state.full(params, chunk).map_err(|e| format!("transcription failed: {e}"))?;
                let n = state.full_n_segments().map_err(|e| e.to_string())?;
                let mut text = String::new();
                for s in 0..n {
                    let seg = state.full_get_segment_text(s).map_err(|e| e.to_string())?;
                    text.push_str(&seg);
                }
                // Whisper sometimes echoes the prompt on near-silent audio.
                let text = text.replace(prompt, "");
                let text = hallucination::collapse_repeats(text.trim());
                if !hallucination::is_hallucination(&text) {
                    pieces.push(text);
                }
            }
            progress((i + 1) as f32 / total as f32);
        }
        Ok(pieces.join(" "))
    }

    /// Auto mode: ask Whisper for language probabilities, then choose between Tamil and English only.
    /// Other languages are ignored, so Tamil is never mis-detected as e.g. Malayalam or Telugu.
    /// If detection fails or is ambiguous, keep the previous chunk's language.
    fn detect_ta_or_en(
        &self,
        state: &mut whisper_rs::WhisperState,
        chunk: &[f32],
        n_threads: i32,
        previous: &'static str,
    ) -> &'static str {
        let (Some(ta), Some(en)) = (whisper_rs::get_lang_id("ta"), whisper_rs::get_lang_id("en")) else {
            return previous;
        };
        if state.pcm_to_mel(chunk, n_threads as usize).is_err() {
            return previous;
        }
        match state.lang_detect(0, n_threads as usize) {
            Ok((_, probs)) => pick_ta_or_en(probs.get(ta as usize).copied(), probs.get(en as usize).copied(), previous),
            Err(_) => previous,
        }
    }
}

/// Choose between Tamil and English from their detection probabilities.
/// A clear winner is needed; near ties keep the previous language so one sentence is not split.
pub fn pick_ta_or_en(ta: Option<f32>, en: Option<f32>, previous: &'static str) -> &'static str {
    match (ta, en) {
        (Some(t), Some(e)) if t > e * 1.15 => "ta",
        (Some(t), Some(e)) if e > t * 1.15 => "en",
        _ => previous,
    }
}

/// Split `audio` into (start, end) ranges of at most `max_len` samples, moving each cut to the
/// quietest 100 ms window within the last 4 seconds of the range so words are not sliced.
pub fn split_chunks(audio: &[f32], max_len: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let win = SAMPLE_RATE / 10;
    let search = 4 * SAMPLE_RATE;
    while start < audio.len() {
        let hard_end = (start + max_len).min(audio.len());
        if hard_end == audio.len() {
            out.push((start, hard_end));
            break;
        }
        let lo = hard_end.saturating_sub(search).max(start + win);
        let mut best_pos = hard_end;
        let mut best_energy = f32::MAX;
        let mut pos = lo;
        while pos + win <= hard_end {
            let e: f32 = audio[pos..pos + win].iter().map(|s| s * s).sum();
            if e < best_energy {
                best_energy = e;
                best_pos = pos + win / 2;
            }
            pos += win / 2;
        }
        out.push((start, best_pos));
        start = best_pos;
    }
    if out.is_empty() {
        out.push((0, 0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_audio_is_one_chunk() {
        let a = vec![0.1f32; SAMPLE_RATE * 5];
        assert_eq!(split_chunks(&a, 28 * SAMPLE_RATE), vec![(0, a.len())]);
    }

    #[test]
    fn long_audio_cut_at_quiet_point_and_covers_everything() {
        let mut a = vec![0.3f32; SAMPLE_RATE * 60];
        // Quiet gap around 26 s.
        for s in a.iter_mut().skip(26 * SAMPLE_RATE).take(SAMPLE_RATE / 2) {
            *s = 0.0;
        }
        let chunks = split_chunks(&a, 28 * SAMPLE_RATE);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].0, 0);
        assert_eq!(chunks.last().unwrap().1, a.len());
        for w in chunks.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
        let first_end = chunks[0].1;
        assert!(first_end >= 26 * SAMPLE_RATE && first_end <= 27 * SAMPLE_RATE, "cut at {first_end}");
        assert!(chunks.iter().all(|(s, e)| e - s <= 28 * SAMPLE_RATE));
    }

    #[test]
    fn auto_language_only_ever_picks_tamil_or_english() {
        assert_eq!(pick_ta_or_en(Some(0.7), Some(0.1), "en"), "ta");
        assert_eq!(pick_ta_or_en(Some(0.1), Some(0.8), "ta"), "en");
        assert_eq!(pick_ta_or_en(Some(0.40), Some(0.41), "ta"), "ta"); // near tie keeps previous
        assert_eq!(pick_ta_or_en(None, Some(0.9), "ta"), "ta");
    }

    #[test]
    fn empty_audio() {
        assert_eq!(split_chunks(&[], 100), vec![(0, 0)]);
    }
}
