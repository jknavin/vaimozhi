use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "tamil" | "english" | "auto"
    pub language: String,
    /// Catalog id of the active model, empty until the user picks one.
    pub model_id: String,
    /// Input device name; empty = system default.
    pub microphone: String,
    pub shortcut: String,
    /// "toggle" (press to start, press to stop) or "push" (hold to talk)
    pub hotkey_mode: String,
    /// raw | clean | professional | formal | casual | email
    pub style: String,
    pub remove_fillers: bool,
    pub spoken_numbers: bool,
    /// "paste" (clipboard + Ctrl+V) | "type" (simulated typing, clipboard untouched) | "copy" (clipboard only)
    pub insert_method: String,
    /// Restore the previous clipboard after pasting.
    pub restore_clipboard: bool,
    pub history_enabled: bool,
    /// Keep the 16 kHz recording on disk so a transcript can be retried. Off by default.
    pub keep_audio: bool,
    /// "dark" | "light" | "system"
    pub theme: String,
    /// "en" | "ta"
    pub ui_language: String,
    pub autostart: bool,
    pub start_hidden: bool,
    pub minimize_to_tray: bool,
    pub show_overlay: bool,
    /// Unload the model from RAM after this many idle minutes (0 = never).
    pub unload_after_min: u32,
    /// 0 = automatic
    pub threads: u32,
    /// Beam search (more accurate, a bit slower). On by default: accuracy is the priority.
    pub beam_search: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: "auto".into(),
            model_id: String::new(),
            microphone: String::new(),
            shortcut: "Ctrl+Shift+Space".into(),
            hotkey_mode: "toggle".into(),
            style: "clean".into(),
            remove_fillers: true,
            spoken_numbers: false,
            insert_method: "paste".into(),
            restore_clipboard: true,
            history_enabled: true,
            keep_audio: false,
            theme: "dark".into(),
            ui_language: "en".into(),
            autostart: false,
            start_hidden: false,
            minimize_to_tray: true,
            show_overlay: true,
            unload_after_min: 10,
            threads: 0,
            beam_search: true,
        }
    }
}
