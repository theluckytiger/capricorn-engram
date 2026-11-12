//! Windows: Win32 for windows and input, WinRT for media sessions, COM for audio and the Start menu's apps.
//!
//! Everything here is blocking; tools call it through `tokio::task::spawn_blocking`.

mod apps;
mod audio;
mod caret;
mod gpu;
mod keys;
mod media;
pub mod ocr;
pub mod registry;
mod screen;
pub mod vault;
pub mod system;
mod uia;
mod wins;

pub use apps::{apps, launch};
pub use audio::{mute, set_mute, set_volume, volume};
pub use caret::caret;
pub use gpu::{GpuMeter, front_fullscreen};
pub use keys::{press, type_text};
pub use media::{media, now_playing};
pub use screen::{Shot, caption_button_at, capture, changed, click_point, drag_points, restore_pointer, screen_shot, window_at, window_at_not_mine};
pub use uia::{Pressed, controls, focus_takes_text, inspect, press_control, scroll_into_view, set_value};
pub use wins::{Place, WinAction, above_taskbar, act, place, alive, click_through, exe_path, foreground, give_back_focus, list_windows, tint_frame, set_click_through, set_shown, shake, wait_for_window};

/// COM must be set up on each thread that uses it. Multithreaded apartment: no message pump needed.
fn com() {
    use ::windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    keep_com_alive();
    // S_FALSE (already initialised on this thread) is fine; a different apartment model is too, for our calls.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// COM's shared apartment stays up for as long as she does. It goes when the last thread in it ends, and a worker thread
/// ends after sitting idle for 10 s: during a long wait for an online brain all of them did, and the next "what's
/// playing?" used a Windows object that had gone with it (an access violation that took the whole program down).
pub(crate) fn keep_com_alive() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: no arguments that can be wrong; the cookie is deliberately never handed back.
        let _ = unsafe { ::windows::Win32::System::Com::CoIncrementMTAUsage() };
    });
}
