//! Putting recognised text into the application the user was typing in.
//!
//! Methods (user-selectable):
//!  * `paste` – put the text on the clipboard, send Ctrl+V, then (optionally) restore the old
//!    clipboard. Text and image clipboards are restored; other formats (files, rich data) cannot
//!    be and will be lost, so users who copy such content often should pick `type` or turn
//!    restoring off.
//!  * `type`  – simulate typing the characters (Unicode, so Tamil works). Clipboard untouched.
//!    Slower for long text and a few apps ignore synthetic Unicode input.
//!  * `copy`  – only place the text on the clipboard.

use arboard::{Clipboard, ImageData};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::time::Duration;

enum Saved {
    Text(String),
    Image(ImageData<'static>),
    Nothing,
}

fn save_clipboard(cb: &mut Clipboard) -> Saved {
    if let Ok(t) = cb.get_text() {
        return Saved::Text(t);
    }
    if let Ok(img) = cb.get_image() {
        return Saved::Image(img.to_owned_img());
    }
    Saved::Nothing
}

fn restore_clipboard(cb: &mut Clipboard, saved: Saved) {
    match saved {
        Saved::Text(t) => {
            let _ = cb.set_text(t);
        }
        Saved::Image(img) => {
            let _ = cb.set_image(img);
        }
        Saved::Nothing => {
            let _ = cb.clear();
        }
    }
}

const VK_V: u32 = 0x56;

fn send_ctrl_v() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("keyboard access failed: {e}"))?;
    enigo.key(Key::Control, Direction::Press).map_err(|e| e.to_string())?;
    // Raw virtual-key code so the shortcut works on any keyboard layout (including Tamil).
    let r = enigo.key(Key::Other(VK_V), Direction::Click);
    let _ = enigo.key(Key::Control, Direction::Release);
    r.map_err(|e| e.to_string())
}

pub fn insert_text(method: &str, restore: bool, text: &str) -> Result<(), String> {
    match method {
        "type" => {
            let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("keyboard access failed: {e}"))?;
            enigo.text(text).map_err(|e| format!("typing failed: {e}"))
        }
        "copy" => {
            let mut cb = Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
            cb.set_text(text.to_string()).map_err(|e| e.to_string())
        }
        _ => {
            let mut cb = Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
            let saved = if restore { Some(save_clipboard(&mut cb)) } else { None };
            cb.set_text(text.to_string()).map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(80));
            let sent = send_ctrl_v();
            // Give the target application time to read the clipboard before we put the old content back.
            std::thread::sleep(Duration::from_millis(450));
            if let Some(saved) = saved {
                restore_clipboard(&mut cb, saved);
            }
            sent
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Win32 helpers: remember which window the user was in, and keep our overlay from stealing focus.
// `as isize` / `as _` casts keep this working whether windows-sys models HWND as isize or a pointer.
// ---------------------------------------------------------------------------------------------

#[cfg(windows)]
pub fn foreground_window() -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe { GetForegroundWindow() as isize }
}

#[cfg(windows)]
pub fn focus_window(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow, SetForegroundWindow};
    if hwnd == 0 {
        return;
    }
    unsafe {
        if IsWindow(hwnd as _) != 0 && GetForegroundWindow() as isize != hwnd {
            SetForegroundWindow(hwnd as _);
        }
    }
}

/// Mark a window as a non-activating tool window (never takes keyboard focus, hidden from Alt+Tab).
#[cfg(windows)]
pub fn make_no_activate(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    unsafe {
        let old = GetWindowLongPtrW(hwnd as _, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd as _, GWL_EXSTYLE, old | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize);
    }
}

#[cfg(windows)]
pub fn show_no_activate(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
    unsafe {
        ShowWindow(hwnd as _, SW_SHOWNOACTIVATE);
    }
}

#[cfg(not(windows))]
pub fn foreground_window() -> isize {
    0
}
#[cfg(not(windows))]
pub fn focus_window(_hwnd: isize) {}
#[cfg(not(windows))]
pub fn make_no_activate(_hwnd: isize) {}
#[cfg(not(windows))]
pub fn show_no_activate(_hwnd: isize) {}
