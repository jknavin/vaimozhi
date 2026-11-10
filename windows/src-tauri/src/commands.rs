//! Tauri commands invoked from the UI.

use crate::dictation::{self, Target};
use crate::settings::Settings;
use crate::{db, export, lk, modelmgr, AppState};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;
use vaimozhi_core::{cleanup, models, CleanupOptions, Style};

type St<'a> = State<'a, Arc<AppState>>;

// ----------------------------------------------------------------------------- settings

#[tauri::command]
pub fn get_settings(st: St) -> Settings {
    lk(&st.settings).clone()
}

#[tauri::command]
pub fn save_settings(app: AppHandle, st: St, settings: Settings) -> Result<Settings, String> {
    let old = lk(&st.settings).clone();

    if settings.shortcut != old.shortcut {
        if let Err(e) = crate::register_shortcut(&app, &settings.shortcut) {
            // Put the old shortcut back so dictation keeps working.
            let _ = crate::register_shortcut(&app, &old.shortcut);
            return Err(e);
        }
    }
    if settings.autostart != old.autostart {
        let al = app.autolaunch();
        let r = if settings.autostart { al.enable() } else { al.disable() };
        r.map_err(|e| format!("Could not change the start-with-Windows option: {e}"))?;
    }
    if settings.model_id != old.model_id {
        *lk(&st.engine) = None; // unload; the new model loads on the next dictation
    }

    let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    db::kv_set(&lk(&st.db), "settings", &json).map_err(|e| e.to_string())?;
    *lk(&st.settings) = settings.clone();
    Ok(settings)
}

#[tauri::command(async)]
pub fn list_microphones() -> Vec<String> {
    crate::audio::list_input_devices()
}

#[derive(Serialize)]
pub struct MicTest {
    ok: bool,
    peak: f32,
    message: String,
}

/// Open the selected microphone for a moment and report whether sound arrives.
#[tauri::command(async)]
pub fn test_microphone(st: St) -> Result<MicTest, String> {
    let device = lk(&st.settings).microphone.clone();
    Ok(match crate::audio::start(&device, |_, _| {}) {
        Err(e) => MicTest { ok: false, peak: 0.0, message: e },
        Ok(rec) => {
            std::thread::sleep(std::time::Duration::from_millis(900));
            match rec.finish() {
                Err(e) => MicTest { ok: false, peak: 0.0, message: e },
                Ok(c) if c.peak < 1e-5 => MicTest {
                    ok: false,
                    peak: c.peak,
                    message: "The microphone opened but delivered silence. Check Windows Settings > Privacy > Microphone, the input volume, and that this device is not muted.".into(),
                },
                Ok(c) => MicTest { ok: true, peak: c.peak, message: "Microphone is working.".into() },
            }
        }
    })
}

// ----------------------------------------------------------------------------- device & storage

#[derive(Serialize)]
pub struct DeviceInfo {
    ram_mb: u64,
    cores: usize,
    recommended_id: String,
    free_disk_mb: Option<u64>,
    data_dir: String,
    models_dir: String,
    models_bytes: u64,
    history_bytes: u64,
    audio_bytes: u64,
    version: String,
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| rd.filter_map(Result::ok).filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

#[tauri::command(async)]
pub fn device_info(st: St) -> Result<DeviceInfo, String> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let ram_mb = sys.total_memory() / 1_048_576;
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let rec = models::recommend(ram_mb, cores);
    let db_path = st.data_dir.join("vaimozhi.db");
    Ok(DeviceInfo {
        ram_mb,
        cores,
        recommended_id: rec.id.into(),
        free_disk_mb: modelmgr::free_space_bytes(&st.models_dir).map(|b| b / 1_048_576),
        data_dir: st.data_dir.to_string_lossy().into_owned(),
        models_dir: st.models_dir.to_string_lossy().into_owned(),
        models_bytes: dir_size(&st.models_dir),
        history_bytes: std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0),
        audio_bytes: dir_size(&st.data_dir.join("audio")),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

// ----------------------------------------------------------------------------- models

#[tauri::command(async)]
pub fn list_models(st: St) -> Result<Vec<modelmgr::ModelStatus>, String> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let rec = models::recommend(sys.total_memory() / 1_048_576, cores);
    Ok(modelmgr::list(&st.models_dir, rec.id))
}

#[derive(Serialize, Clone)]
struct ModelProgress {
    id: String,
    state: &'static str,
    done: u64,
    total: u64,
    message: String,
}

#[tauri::command]
pub fn download_model(app: AppHandle, st: St, id: String) -> Result<(), String> {
    let spec = models::find(&id).ok_or("unknown model")?;
    let cancel = {
        let mut map = lk(&st.downloads);
        if map.contains_key(&id) {
            return Err("This model is already downloading.".into());
        }
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        map.insert(id.clone(), flag.clone());
        flag
    };
    let st = st.inner().clone();
    std::thread::spawn(move || {
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let id_p = id.clone();
        let app_p = app.clone();
        let result = modelmgr::download(&st.models_dir, spec, &cancel, |done, total| {
            if last.elapsed() >= std::time::Duration::from_millis(250) {
                last = std::time::Instant::now();
                let _ = app_p.emit(
                    "model-progress",
                    ModelProgress { id: id_p.clone(), state: "downloading", done, total, message: String::new() },
                );
            }
        });
        lk(&st.downloads).remove(&id);
        let payload = match result {
            Ok(v) => ModelProgress { id: id.clone(), state: "done", done: 0, total: 0, message: v.message },
            Err(e) if e == "cancelled" => {
                ModelProgress { id: id.clone(), state: "cancelled", done: 0, total: 0, message: "Download paused. It can be resumed.".into() }
            }
            Err(e) => ModelProgress { id: id.clone(), state: "error", done: 0, total: 0, message: e },
        };
        let _ = app.emit("model-progress", payload);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_download(st: St, id: String) {
    if let Some(flag) = lk(&st.downloads).get(&id) {
        flag.store(true, Ordering::SeqCst);
    }
}

#[tauri::command(async)]
pub fn delete_model(st: St, id: String) -> Result<(), String> {
    let spec = models::find(&id).ok_or("unknown model")?;
    if lk(&st.engine).as_ref().map(|e| e.model_id == id).unwrap_or(false) {
        *lk(&st.engine) = None; // release the file handle before deleting
    }
    modelmgr::delete(&st.models_dir, spec).map_err(|e| format!("Could not delete: {e}"))
}

#[tauri::command(async)]
pub fn verify_model(st: St, id: String) -> Result<modelmgr::VerifyResult, String> {
    let spec = models::find(&id).ok_or("unknown model")?;
    Ok(modelmgr::verify(&st.models_dir, spec))
}

// ----------------------------------------------------------------------------- recording (UI)

#[tauri::command(async)]
pub fn start_recording(app: AppHandle) -> Result<(), String> {
    dictation::start(&app, Target::Editor)
}
#[tauri::command(async)]
pub fn stop_recording(app: AppHandle) {
    dictation::stop(&app)
}
#[tauri::command]
pub fn pause_recording(app: AppHandle) {
    dictation::set_paused(&app, true)
}
#[tauri::command]
pub fn resume_recording(app: AppHandle) {
    dictation::set_paused(&app, false)
}
#[tauri::command]
pub fn cancel_recording(app: AppHandle) {
    dictation::cancel(&app)
}
#[tauri::command]
pub fn cancel_transcription(st: St) {
    st.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command(async)]
pub fn retranscribe(app: AppHandle, id: i64, language: String) -> Result<(), String> {
    dictation::retranscribe(&app, id, language)
}

#[tauri::command]
pub fn clean_text(st: St, text: String, style: String) -> String {
    let s = lk(&st.settings).clone();
    cleanup(
        &text,
        &CleanupOptions { style: Style::parse(&style), remove_fillers: s.remove_fillers, spoken_numbers: s.spoken_numbers },
    )
}

// ----------------------------------------------------------------------------- history

#[derive(Serialize)]
pub struct Stats {
    words: i64,
    count: i64,
}

#[tauri::command]
pub fn get_stats(st: St) -> Stats {
    let (words, count) = db::stats(&lk(&st.db));
    Stats { words, count }
}

#[tauri::command]
pub fn list_history(
    st: St,
    query: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    favorites_only: Option<bool>,
    limit: Option<i64>,
) -> Result<Vec<db::Transcript>, String> {
    db::list(&lk(&st.db), query.as_deref(), from_ms, to_ms, favorites_only.unwrap_or(false), limit.unwrap_or(200))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_transcript(st: St, id: i64) -> Option<db::Transcript> {
    db::get(&lk(&st.db), id)
}

#[tauri::command]
pub fn save_transcript(
    st: St,
    text: String,
    raw_text: String,
    language: String,
    model: String,
    style: String,
    duration_ms: i64,
) -> Result<i64, String> {
    db::insert(
        &lk(&st.db),
        &db::NewTranscript {
            title: "",
            text: &text,
            raw_text: &raw_text,
            language: &language,
            model: &model,
            style: &style,
            duration_ms,
            audio_path: None,
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_transcript(st: St, id: i64, text: String) -> Result<(), String> {
    db::update_text(&lk(&st.db), id, &text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_transcript(st: St, id: i64, title: String) -> Result<(), String> {
    db::rename(&lk(&st.db), id, &title).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_favorite(st: St, id: i64, favorite: bool) -> Result<(), String> {
    db::set_favorite(&lk(&st.db), id, favorite).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_transcript(st: St, id: i64) -> Result<(), String> {
    let audio = db::delete(&lk(&st.db), id).map_err(|e| e.to_string())?;
    if let Some(p) = audio {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

#[tauri::command]
pub fn clear_history(st: St) -> Result<(), String> {
    let paths = db::clear(&lk(&st.db)).map_err(|e| e.to_string())?;
    for p in paths {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

#[tauri::command(async)]
pub fn export_transcript(st: St, id: i64, format: String, path: String) -> Result<(), String> {
    let t = db::get(&lk(&st.db), id).ok_or("transcript not found")?;
    let p = std::path::Path::new(&path);
    match format.as_str() {
        "txt" => export::export_txt(p, &t.title, &t.text),
        "docx" => export::export_docx(p, &t.title, &t.text),
        other => Err(format!("unsupported export format: {other}")),
    }
}
