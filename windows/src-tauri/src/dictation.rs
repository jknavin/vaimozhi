//! The dictation pipeline: record → transcribe → finish language → cleanup → insert → save history.

use crate::{asr, audio, db, insert, lk, modelmgr, AppState};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use vaimozhi_core::{cleanup, models, CleanupOptions, Language, Style};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// Hotkey / tray: insert into the active application.
    Paste,
    /// Mic button in the app: show the result in the editor.
    Editor,
}

impl Target {
    fn name(&self) -> &'static str {
        match self {
            Target::Paste => "paste",
            Target::Editor => "editor",
        }
    }
}

#[derive(Serialize, Clone)]
struct StatusPayload {
    state: &'static str,
    message: Option<String>,
    target: &'static str,
}

#[derive(Serialize, Clone)]
struct LevelPayload {
    level: f32,
    elapsed_ms: u64,
}

#[derive(Serialize, Clone)]
pub struct ResultPayload {
    pub id: Option<i64>,
    pub text: String,
    pub raw_text: String,
    pub language: String,
    pub style: String,
    pub model: String,
    pub duration_ms: i64,
    pub target: &'static str,
}

fn state(app: &AppHandle) -> Arc<AppState> {
    app.state::<Arc<AppState>>().inner().clone()
}

pub fn emit_status(app: &AppHandle, state: &'static str, message: Option<String>, target: Target) {
    let _ = app.emit("status", StatusPayload { state, message, target: target.name() });
}

pub fn is_recording(app: &AppHandle) -> bool {
    let st = state(app);
    let rec = lk(&st.rec);
    rec.is_some()
}

/// Called by the global shortcut for both key-down and key-up.
pub fn on_hotkey(app: &AppHandle, pressed: bool) {
    let st = state(app);
    let mode = lk(&st.settings).hotkey_mode.clone();
    if pressed {
        {
            let mut down = lk(&st.hotkey_down);
            // Ignore OS key-repeat while the key is held (but never stay stuck if a release is missed).
            if down.0 && down.1.elapsed() < Duration::from_secs(2) {
                return;
            }
            *down = (true, Instant::now());
        }
        let app = app.clone();
        std::thread::spawn(move || {
            if mode == "push" {
                if !is_recording(&app) {
                    let _ = start(&app, Target::Paste);
                }
            } else {
                let _ = toggle(&app, Target::Paste);
            }
        });
    } else {
        lk(&st.hotkey_down).0 = false;
        if mode == "push" && is_recording(app) {
            let app = app.clone();
            std::thread::spawn(move || stop(&app));
        }
    }
}

pub fn toggle(app: &AppHandle, target: Target) -> Result<(), String> {
    if is_recording(app) {
        stop(app);
        Ok(())
    } else {
        start(app, target)
    }
}

pub fn start(app: &AppHandle, target: Target) -> Result<(), String> {
    let st = state(app);
    if st.busy.load(Ordering::SeqCst) {
        let msg = "Still transcribing the previous recording.".to_string();
        emit_status(app, "error", Some(msg.clone()), target);
        return Err(msg);
    }
    let s = lk(&st.settings).clone();
    let installed = models::find(&s.model_id)
        .map(|spec| modelmgr::path_for(&st.models_dir, spec).exists())
        .unwrap_or(false);
    if !installed {
        let msg = "No speech model is installed yet. Open Models and download one.".to_string();
        emit_status(app, "no-model", Some(msg.clone()), target);
        crate::show_main_window(app);
        return Err(msg);
    }

    let mut rec = lk(&st.rec);
    if rec.is_some() {
        return Ok(());
    }
    if target == Target::Paste {
        st.prev_hwnd.store(insert::foreground_window(), Ordering::SeqCst);
    }
    let app_tick = app.clone();
    let recording = audio::start(&s.microphone, move |level, elapsed_ms| {
        let _ = app_tick.emit("audio-level", LevelPayload { level, elapsed_ms });
    })
    .map_err(|e| {
        emit_status(app, "error", Some(e.clone()), target);
        if target == Target::Paste {
            crate::overlay_flash(app, 4);
        }
        e
    })?;
    *rec = Some(recording);
    *lk(&st.target) = target;
    drop(rec);

    emit_status(app, "recording", None, target);
    if s.show_overlay && target == Target::Paste {
        crate::overlay_show(app);
    }
    Ok(())
}

pub fn set_paused(app: &AppHandle, paused: bool) {
    let st = state(app);
    let target = *lk(&st.target);
    let guard = lk(&st.rec);
    if let Some(r) = guard.as_ref() {
        r.set_paused(paused);
        emit_status(app, if paused { "paused" } else { "recording" }, None, target);
    }
}

pub fn cancel(app: &AppHandle) {
    let st = state(app);
    let target = *lk(&st.target);
    // Dropping the Recording stops the audio thread and discards the samples.
    let had = lk(&st.rec).take().is_some();
    if had {
        crate::overlay_hide(app);
        emit_status(app, "idle", None, target);
    }
}

pub fn stop(app: &AppHandle) {
    let st = state(app);
    let target = *lk(&st.target);
    let Some(recording) = lk(&st.rec).take() else { return };
    let app = app.clone();
    std::thread::spawn(move || process(app, recording, target));
}

fn process(app: AppHandle, recording: audio::Recording, target: Target) {
    let st = state(&app);
    st.busy.store(true, Ordering::SeqCst);
    st.cancel.store(false, Ordering::SeqCst);
    emit_status(&app, "transcribing", None, target);

    let result = (|| -> Result<ResultPayload, String> {
        let cap = recording.finish()?;
        if cap.seconds < 0.4 {
            return Err("Recording was too short.".into());
        }
        if cap.peak < 1e-5 {
            return Err("The microphone delivered only silence. Check Windows Settings > Privacy > Microphone (allow desktop apps) and the selected input device.".into());
        }
        let (raw, text, model, lang, style) = transcribe_core(&app, &st, &cap.samples, None)?;
        if text.trim().is_empty() {
            return Err("No speech was recognised.".into());
        }
        let duration_ms = (cap.seconds * 1000.0) as i64;
        let s = lk(&st.settings).clone();

        if target == Target::Paste {
            crate::overlay_hide(&app);
            insert::focus_window(st.prev_hwnd.load(Ordering::SeqCst));
            std::thread::sleep(Duration::from_millis(120));
            insert::insert_text(&s.insert_method, s.restore_clipboard, &text)?;
        }

        let id = if s.history_enabled {
            let audio_path = if s.keep_audio { save_audio(&st, &cap.samples) } else { None };
            let conn = lk(&st.db);
            db::insert(
                &conn,
                &db::NewTranscript {
                    title: "",
                    text: &text,
                    raw_text: &raw,
                    language: &lang,
                    model: &model,
                    style: &style,
                    duration_ms,
                    audio_path: audio_path.as_deref(),
                },
            )
            .ok()
        } else {
            None
        };

        Ok(ResultPayload { id, text, raw_text: raw, language: lang, style, model, duration_ms, target: target.name() })
    })();

    st.busy.store(false, Ordering::SeqCst);
    match result {
        Ok(payload) => {
            crate::overlay_hide(&app);
            let _ = app.emit("dictation-result", payload);
            emit_status(&app, "idle", None, target);
        }
        Err(e) => {
            emit_status(&app, "error", Some(e), target);
            if target == Target::Paste {
                crate::overlay_flash(&app, 4);
            } else {
                crate::overlay_hide(&app);
            }
        }
    }
}

fn save_audio(st: &AppState, samples: &[f32]) -> Option<String> {
    let dir = st.data_dir.join("audio");
    std::fs::create_dir_all(&dir).ok()?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    let path = dir.join(format!("{ms}.wav"));
    audio::write_wav(&path, samples).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// Returns (raw text, final text, model id, language id, style id).
fn transcribe_core(
    app: &AppHandle,
    st: &Arc<AppState>,
    samples: &[f32],
    language_override: Option<&str>,
) -> Result<(String, String, String, String, String), String> {
    let s = lk(&st.settings).clone();
    let lang = Language::parse(language_override.unwrap_or(&s.language));
    let spec = models::find(&s.model_id).ok_or("No speech model selected. Open Models and choose one.")?;
    let path = modelmgr::path_for(&st.models_dir, spec);
    if !path.exists() {
        return Err("The selected model file is missing. Download it again in Models.".into());
    }

    let mut engine = lk(&st.engine);
    if engine.as_ref().map(|e| e.model_id != s.model_id).unwrap_or(true) {
        *engine = None; // free the old model before loading the new one
        emit_status(app, "loading-model", None, *lk(&st.target));
        *engine = Some(asr::Engine::load(&path, &s.model_id)?);
    }
    *lk(&st.last_used) = Instant::now();

    let cleaned_audio = audio::preprocess(samples);
    let app_progress = app.clone();
    let raw = engine.as_ref().unwrap().transcribe(
        &cleaned_audio,
        lang,
        s.threads,
        s.beam_search,
        &st.cancel,
        move |p| {
            let _ = app_progress.emit("transcribe-progress", p);
        },
    )?;
    *lk(&st.last_used) = Instant::now();
    drop(engine);

    let style = Style::parse(&s.style);
    let text = cleanup(
        &raw,
        &CleanupOptions { style, remove_fillers: s.remove_fillers, spoken_numbers: s.spoken_numbers },
    );
    Ok((raw, text, s.model_id.clone(), lang.id().to_string(), style.id().to_string()))
}

/// Re-run recognition on a stored recording (only possible when "keep audio" was on).
pub fn retranscribe(app: &AppHandle, id: i64, language: String) -> Result<(), String> {
    let st = state(app);
    if st.busy.load(Ordering::SeqCst) {
        return Err("Another transcription is still running.".into());
    }
    let path = db::audio_path(&lk(&st.db), id).ok_or(
        "This transcript has no stored audio. Turn on 'Keep audio for retry' in Settings > Privacy before dictating.",
    )?;
    let samples = audio::read_wav(std::path::Path::new(&path))?;
    let app = app.clone();
    std::thread::spawn(move || {
        let st = state(&app);
        st.busy.store(true, Ordering::SeqCst);
        st.cancel.store(false, Ordering::SeqCst);
        emit_status(&app, "transcribing", None, Target::Editor);
        let r = transcribe_core(&app, &st, &samples, Some(&language));
        st.busy.store(false, Ordering::SeqCst);
        match r {
            Ok((raw, text, model, lang, style)) => {
                {
                    let conn = lk(&st.db);
                    let _ = db::update_text(&conn, id, &text);
                    let _ = conn.execute(
                        "UPDATE transcripts SET raw_text = ?1, language = ?2, model = ?3, style = ?4 WHERE id = ?5",
                        rusqlite::params![raw, lang, model, style, id],
                    );
                }
                let _ = app.emit(
                    "dictation-result",
                    ResultPayload {
                        id: Some(id),
                        text,
                        raw_text: raw,
                        language: lang,
                        style,
                        model,
                        duration_ms: (samples.len() as i64) * 1000 / asr::SAMPLE_RATE as i64,
                        target: "editor",
                    },
                );
                emit_status(&app, "idle", None, Target::Editor);
            }
            Err(e) => emit_status(&app, "error", Some(e), Target::Editor),
        }
    });
    Ok(())
}
