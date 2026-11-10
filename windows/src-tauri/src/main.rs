#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod asr;
mod audio;
mod commands;
mod db;
mod dictation;
mod export;
mod insert;
mod modelmgr;
mod settings;

use settings::Settings;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub settings: Mutex<Settings>,
    pub rec: Mutex<Option<audio::Recording>>,
    pub engine: Mutex<Option<asr::Engine>>,
    pub last_used: Mutex<Instant>,
    pub busy: AtomicBool,
    pub cancel: AtomicBool,
    pub target: Mutex<dictation::Target>,
    pub prev_hwnd: AtomicIsize,
    pub hotkey_down: Mutex<(bool, Instant)>,
    pub downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub data_dir: PathBuf,
    pub models_dir: PathBuf,
}

/// Lock a mutex, recovering the data if another thread panicked while holding it.
pub fn lk<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn register_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    let sc: Shortcut = shortcut
        .parse()
        .map_err(|e| format!("'{shortcut}' is not a valid shortcut ({e}). Example: Ctrl+Shift+Space"))?;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(sc).map_err(|e| format!("Cannot use {shortcut}: {e}. Another program may already use it."))
}

// ----------------------------------------------------------------------------- overlay window

fn overlay_hwnd(app: &AppHandle) -> Option<isize> {
    #[cfg(windows)]
    {
        let w = app.get_webview_window("overlay")?;
        return w.hwnd().ok().map(|h| h.0 as isize);
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

pub fn overlay_show(app: &AppHandle) {
    let Some(w) = app.get_webview_window("overlay") else { return };
    if let Ok(Some(m)) = w.primary_monitor() {
        let scale = m.scale_factor();
        let size = m.size();
        let pos = m.position();
        let ow = (300.0 * scale) as i32;
        let oh = (72.0 * scale) as i32;
        let x = pos.x + (size.width as i32 - ow) / 2;
        let y = pos.y + size.height as i32 - oh - (96.0 * scale) as i32;
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    }
    match overlay_hwnd(app) {
        Some(h) => insert::show_no_activate(h),
        None => {
            let _ = w.show();
        }
    }
}

pub fn overlay_hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.hide();
    }
}

/// Show the overlay for a few seconds so an error is visible even when the app window is hidden.
pub fn overlay_flash(app: &AppHandle, seconds: u64) {
    overlay_show(app);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(seconds));
        let st = app.state::<Arc<AppState>>();
        if !dictation::is_recording(&app) && !st.busy.load(std::sync::atomic::Ordering::SeqCst) {
            overlay_hide(&app);
        }
    });
}

fn create_overlay(app: &AppHandle) -> tauri::Result<()> {
    let w = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("overlay.html".into()))
        .title("VAIMOZHI recording")
        .inner_size(300.0, 72.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .build()?;
    let _ = w.set_ignore_cursor_events(true);
    if let Some(h) = overlay_hwnd(app) {
        insert::make_no_activate(h);
    }
    Ok(())
}

// ----------------------------------------------------------------------------- tray

fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Open VAIMOZHI", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Start / stop dictation", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &toggle, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("VAIMOZHI \u{2013} voice typing")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "toggle" => {
                let app = app.clone();
                std::thread::spawn(move || {
                    let _ = dictation::toggle(&app, dictation::Target::Paste);
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

// ----------------------------------------------------------------------------- idle model unloading

fn spawn_idle_unloader(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(30));
        let st = app.state::<Arc<AppState>>();
        let minutes = lk(&st.settings).unload_after_min;
        if minutes == 0 || st.busy.load(std::sync::atomic::Ordering::SeqCst) {
            continue;
        }
        let idle = lk(&st.last_used).elapsed();
        if idle >= Duration::from_secs(minutes as u64 * 60) {
            let mut engine = lk(&st.engine);
            if engine.is_some() {
                *engine = None;
            }
        }
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    dictation::on_hotkey(app, event.state() == ShortcutState::Pressed);
                })
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let models_dir = data_dir.join("models");
            std::fs::create_dir_all(&models_dir)?;

            let conn = db::open(&data_dir.join("vaimozhi.db"))?;
            let settings: Settings = db::kv_get(&conn, "settings")
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            let mut settings = settings;
            // Old values such as "tanglish" or "mixed" are retired; map them to a supported mode.
            settings.language = vaimozhi_core::Language::parse(&settings.language).id().to_string();

            let state = Arc::new(AppState {
                db: Mutex::new(conn),
                settings: Mutex::new(settings.clone()),
                rec: Mutex::new(None),
                engine: Mutex::new(None),
                last_used: Mutex::new(Instant::now()),
                busy: AtomicBool::new(false),
                cancel: AtomicBool::new(false),
                target: Mutex::new(dictation::Target::Paste),
                prev_hwnd: AtomicIsize::new(0),
                hotkey_down: Mutex::new((false, Instant::now())),
                downloads: Mutex::new(HashMap::new()),
                data_dir,
                models_dir,
            });
            app.manage(state);

            let handle = app.handle().clone();
            create_overlay(&handle)?;
            create_tray(&handle)?;
            spawn_idle_unloader(handle.clone());

            // A taken shortcut must not prevent the app from starting; the UI shows the problem instead.
            if let Err(e) = register_shortcut(&handle, &settings.shortcut) {
                eprintln!("{e}");
            }

            let hidden = settings.start_hidden || std::env::args().any(|a| a == "--hidden");
            if !hidden {
                show_main_window(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let minimize = lk(&window.app_handle().state::<Arc<AppState>>().settings).minimize_to_tray;
                    if minimize {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::list_microphones,
            commands::test_microphone,
            commands::device_info,
            commands::list_models,
            commands::download_model,
            commands::cancel_download,
            commands::delete_model,
            commands::verify_model,
            commands::start_recording,
            commands::stop_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::cancel_recording,
            commands::cancel_transcription,
            commands::retranscribe,
            commands::clean_text,
            commands::get_stats,
            commands::list_history,
            commands::get_transcript,
            commands::save_transcript,
            commands::update_transcript,
            commands::rename_transcript,
            commands::set_favorite,
            commands::delete_transcript,
            commands::clear_history,
            commands::export_transcript,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VAIMOZHI");
}
