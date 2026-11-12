//! Where your text cursor is, on screen: the window in front's caret, the way Windows' Magnifier follows it. Classic
//! apps keep a system caret (`GetGUIThreadInfo`); browsers and Electron apps publish theirs for screen readers as the
//! window's caret object (`OBJID_CARET`).

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{AccessibleObjectFromWindow, IAccessible};
use windows::Win32::UI::WindowsAndMessaging::{GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, OBJID_CARET};
use windows::core::Interface;

/// The text cursor in the window in front, as left, top, right, bottom in screen pixels; `None` when there's none
/// (no text box has the focus) or the window in front is one of hers.
pub fn caret() -> Option<[i32; 4]> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        let thread = GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return None;
        }
        let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if GetGUIThreadInfo(thread, &mut info).is_ok() && !info.hwndCaret.0.is_null() && info.rcCaret.bottom > info.rcCaret.top {
            let r = info.rcCaret;
            let mut a = POINT { x: r.left, y: r.top };
            let mut b = POINT { x: r.right.max(r.left + 1), y: r.bottom };
            if ClientToScreen(info.hwndCaret, &mut a).as_bool() && ClientToScreen(info.hwndCaret, &mut b).as_bool() {
                return Some([a.x, a.y, b.x, b.y]);
            }
        }
        accessible_caret(fg)
    }
}

/// The caret object a browser or Electron app publishes (COM is set up on this thread as needed).
unsafe fn accessible_caret(window: HWND) -> Option<[i32; 4]> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let mut obj: *mut std::ffi::c_void = std::ptr::null_mut();
        AccessibleObjectFromWindow(window, OBJID_CARET.0 as u32, &IAccessible::IID, &mut obj).ok()?;
        if obj.is_null() {
            return None;
        }
        let acc = IAccessible::from_raw(obj);
        let (mut x, mut y, mut w, mut h) = (0i32, 0i32, 0i32, 0i32);
        acc.accLocation(&mut x, &mut y, &mut w, &mut h, &VARIANT::from(0i32)).ok()?;
        // No caret: an empty rectangle, often at 0,0.
        if h <= 0 || (x == 0 && y == 0) {
            return None;
        }
        let r = RECT { left: x, top: y, right: x + w.max(1), bottom: y + h };
        Some([r.left, r.top, r.right, r.bottom])
    }
}
