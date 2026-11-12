//! Top-level windows: listing them, and focusing, minimizing, maximizing, restoring, closing.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, PWSTR};

use super::wide;
use crate::platform::WinInfo;

/// Exe path → friendly name, so version resources are read once per program.
static NAMES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Visible, real top-level windows, front to back (the order Windows enumerates them in).
pub fn list_windows() -> Vec<WinInfo> {
    let mut handles: Vec<HWND> = Vec::new();
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the &mut Vec we passed below, alive for the whole EnumWindows call.
        let handles = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        handles.push(hwnd);
        BOOL(1)
    }
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    let fg = unsafe { GetForegroundWindow() };
    handles.into_iter().filter_map(|h| info(h, fg)).collect()
}

fn info(hwnd: HWND, fg: HWND) -> Option<WinInfo> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return None;
        }
        // UWP windows that exist but aren't shown ("cloaked") would otherwise look open.
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        if cloaked != 0 {
            return None;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        // Invisible characters out (Call of Duty puts zero-width spaces in its title): names are matched by what shows.
        let title: String = String::from_utf16_lossy(&buf[..n as usize]).chars().filter(|c| !matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FEFF}')).collect();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let path = exe_path(pid).unwrap_or_default();
        let exe = Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let app = friendly_name(&path).unwrap_or_else(|| exe.clone());
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        Some(WinInfo {
            id: hwnd.0 as isize,
            title,
            app,
            exe,
            pid,
            rect: [r.left, r.top, r.right, r.bottom],
            minimized: IsIconic(hwnd).as_bool(),
            maximized: style & WS_MAXIMIZE.0 != 0,
            foreground: hwnd == fg,
        })
    }
}

/// Whether a process is still running.
pub fn alive(pid: u32) -> bool {
    use windows::Win32::System::Threading::GetExitCodeProcess;
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = CloseHandle(h);
        // 259 = STILL_ACTIVE
        ok && code == 259
    }
}

/// The program's full path.
pub fn exe_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// The program's FileDescription ("Discord", "Brave Browser"), which beats exe names like "steamwebhelper".
fn friendly_name(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let mut cache = NAMES.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(n) = cache.get(path) {
        return Some(n.clone());
    }
    let name = unsafe { file_description(path) }.filter(|d| !d.trim().is_empty());
    let name = name.map(|d| d.trim().to_string());
    if let Some(n) = &name {
        cache.insert(path.to_string(), n.clone());
    }
    name
}

unsafe fn file_description(path: &str) -> Option<String> {
    let wpath = wide(path);
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wpath.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wpath.as_ptr()), None, size, data.as_mut_ptr() as *mut _).ok()?;
        // The first language/codepage pair, then its FileDescription.
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let q = wide("\\VarFileInfo\\Translation");
        if !VerQueryValueW(data.as_ptr() as *const _, PCWSTR(q.as_ptr()), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let pair = *(ptr as *const [u16; 2]);
        let key = wide(&format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", pair[0], pair[1]));
        if !VerQueryValueW(data.as_ptr() as *const _, PCWSTR(key.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        let s = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        Some(String::from_utf16_lossy(s).trim_end_matches('\0').to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinAction {
    Focus,
    Minimize,
    Maximize,
    Restore,
    Close,
    /// Ends the app's process (asked first: anything unsaved is lost). Never Windows' own parts.
    ForceClose,
}

/// Windows' own processes: never force quit.
const WINDOWS_OWN: &[&str] = &["explorer", "dwm", "csrss", "winlogon", "wininit", "svchost", "lsass", "services", "smss", "system", "sihost", "taskmgr", "searchhost", "startmenuexperiencehost", "shellexperiencehost", "textinputhost", "ctfmon", "fontdrvhost", "applicationframehost", "runtimebroker"];

/// Does it, then checks that it happened. Returns what was seen afterwards.
pub fn act(id: isize, action: WinAction) -> Result<String> {
    let hwnd = HWND(id as *mut _);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            bail!("that window is gone");
        }
        match action {
            WinAction::Focus => {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                forward(hwnd);
                if !settle(|| GetForegroundWindow() == hwnd) {
                    bail!("Windows didn't let it come to the front");
                }
                Ok("it's in front now".into())
            }
            WinAction::Minimize => {
                let _ = ShowWindowAsync(hwnd, SW_MINIMIZE);
                if !settle(|| IsIconic(hwnd).as_bool()) {
                    bail!("it didn't minimize");
                }
                Ok("minimized".into())
            }
            WinAction::Maximize => {
                let _ = ShowWindowAsync(hwnd, SW_MAXIMIZE);
                if !settle(|| (GetWindowLongW(hwnd, GWL_STYLE) as u32) & WS_MAXIMIZE.0 != 0) {
                    bail!("it didn't maximize");
                }
                Ok("maximized".into())
            }
            WinAction::Restore => {
                let _ = ShowWindowAsync(hwnd, SW_RESTORE);
                if !settle(|| !IsIconic(hwnd).as_bool() && (GetWindowLongW(hwnd, GWL_STYLE) as u32) & WS_MAXIMIZE.0 == 0) {
                    bail!("it didn't restore");
                }
                Ok("restored".into())
            }
            WinAction::ForceClose => {
                use windows::Win32::System::Threading::{PROCESS_TERMINATE, TerminateProcess};
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let exe = exe_path(pid).map(|p| Path::new(&p).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()).unwrap_or_default();
                if pid == 0 || pid == std::process::id() || WINDOWS_OWN.contains(&exe.as_str()) {
                    bail!("that's part of Windows (or her): not something to force quit");
                }
                let h = OpenProcess(PROCESS_TERMINATE, false, pid)?;
                let done = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
                done?;
                if !settle(|| !IsWindow(Some(hwnd)).as_bool()) {
                    bail!("it was told to end but its window is still there");
                }
                Ok("force quit".into())
            }
            WinAction::Close => {
                // Like its own close button: the app may still ask to save.
                PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(2500) {
                    if !IsWindow(Some(hwnd)).as_bool() || !IsWindowVisible(hwnd).as_bool() {
                        return Ok("closed".into());
                    }
                    std::thread::sleep(Duration::from_millis(80));
                }
                // Still there: maybe a "save changes?" dialog, maybe it hid to the tray. Say what's in front.
                let fg = GetForegroundWindow();
                let mut buf = [0u16; 256];
                let n = GetWindowTextW(fg, &mut buf);
                bail!("it's still open; in front now: \"{}\" (it may be asking to save)", String::from_utf16_lossy(&buf[..n as usize]))
            }
        }
    }
}

/// Hides or shows a window without waiting on the thread that owns it (that thread may be stuck). Shown again without
/// taking the focus.
pub fn set_shown(id: isize, shown: bool) {
    let _ = unsafe { ShowWindowAsync(HWND(id as *mut _), if shown { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
}

/// Lets the mouse through a window (`on`), or has it catch the mouse, without activating it. (The window library's
/// own switch also shows the window "activated": every time your pointer touched her, the app you were typing in
/// lost the keyboard.)
pub fn set_click_through(id: isize, on: bool) {
    let hwnd = HWND(id as *mut _);
    let bits = (WS_EX_TRANSPARENT | WS_EX_LAYERED).0 as isize;
    unsafe {
        let now = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = if on { now | bits } else { now & !bits };
        if want != now {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
            let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_ASYNCWINDOWPOS);
        }
    }
}

/// Paints a window's title bar, its title and its border (colours as 0xRRGGBB) the way Windows 11 allows; older
/// Windows keeps its own. Her settings window had Windows' grey title bar over her dark purple.
pub fn tint_frame(id: isize, caption: u32, text: u32, border: u32) {
    use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DwmSetWindowAttribute};
    let hwnd = HWND(id as *mut _);
    // Windows takes 0x00BBGGRR.
    let bgr = |rgb: u32| ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff);
    for (attr, rgb) in [(DWMWA_CAPTION_COLOR, caption), (DWMWA_TEXT_COLOR, text), (DWMWA_BORDER_COLOR, border)] {
        let v = bgr(rgb);
        let _ = unsafe { DwmSetWindowAttribute(hwnd, attr, (&v as *const u32).cast(), 4) };
    }
}

/// The window you're in (it has the keyboard).
pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Gives the keyboard back to window `id` (her windows took it by appearing).
pub fn give_back_focus(id: isize) {
    if id != 0 {
        let _ = unsafe { SetForegroundWindow(HWND(id as *mut _)) };
    }
}

/// Whether a window lets the mouse through.
pub fn click_through(id: isize) -> bool {
    unsafe { GetWindowLongPtrW(HWND(id as *mut _), GWL_EXSTYLE) & WS_EX_TRANSPARENT.0 as isize != 0 }
}

/// Shakes a window where it is (she hit it, `power` 0..1): a few quick nudges, then back exactly where it was. Not a
/// maximized one.
pub fn shake(id: isize, power: f64) {
    let hwnd = HWND(id as *mut _);
    unsafe {
        let mut r = RECT::default();
        if !IsWindow(Some(hwnd)).as_bool() || IsZoomed(hwnd).as_bool() || GetWindowRect(hwnd, &mut r).is_err() {
            return;
        }
        let d = (4.0 + power.clamp(0.0, 1.0) * 14.0) as i32;
        for (dx, dy) in [(0, d), (d / 2, -d / 3), (-d / 2, d / 3), (d / 3, -d / 4), (0, 0)] {
            let _ = SetWindowPos(hwnd, None, r.left + dx, r.top + dy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

/// Polls a condition for up to a second.
fn settle(ok: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1000) {
        if ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    ok()
}

/// Waits for a window of a process (by exe name or app name) to show up.
pub fn wait_for_window(matches: impl Fn(&WinInfo) -> bool, timeout: Duration) -> Option<WinInfo> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(w) = list_windows().into_iter().find(|w| matches(w)) {
            return Some(w);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    None
}

/// Brings `hwnd` to the front without a key press. Windows only lets the app that last had input do that: joining the
/// front window's input for a moment allows it, and a mouse nudge of zero pixels counts as input if that wasn't enough.
/// (It used to tap Alt: in a browser a lone Alt puts the keyboard on its menu button, and what she typed next went
/// nowhere.)
fn forward(hwnd: HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    unsafe {
        let front = GetForegroundWindow();
        let theirs = GetWindowThreadProcessId(front, None);
        let mine = GetCurrentThreadId();
        let joined = theirs != 0 && theirs != mine && AttachThreadInput(mine, theirs, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if joined {
            let _ = AttachThreadInput(mine, theirs, false);
        }
        if GetForegroundWindow() != hwnd {
            let nudge = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: MOUSEEVENTF_MOVE, time: 0, dwExtraInfo: 0 } } };
            SendInput(&[nudge], std::mem::size_of::<INPUT>() as i32);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Her overlay back above the taskbar, if a taskbar has come up over it: the taskbar is always-on-top too, and
/// clicking it puts it at the top of that band, over her sitting on it. Only the taskbar: other always-on-top windows
/// keep their place. Never takes the focus.
pub fn above_taskbar(id: isize) {
    let me = HWND(id as *mut _);
    unsafe {
        let mut h = GetWindow(me, GW_HWNDPREV).unwrap_or_default();
        while !h.0.is_null() {
            let mut class = [0u16; 64];
            let n = GetClassNameW(h, &mut class) as usize;
            let name = String::from_utf16_lossy(&class[..n]);
            if (name == "Shell_TrayWnd" || name == "Shell_SecondaryTrayWnd") && IsWindowVisible(h).as_bool() {
                let _ = SetWindowPos(me, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS);
                return;
            }
            h = GetWindow(h, GW_HWNDPREV).unwrap_or_default();
        }
    }
}

/// Where a window goes: half of its screen (left, right, top, bottom), or kept above the rest (or not any more).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Left,
    Right,
    Top,
    Bottom,
    OnTop,
    NotOnTop,
}

/// Puts a window there (out of maximized first, the taskbar left clear).
pub fn place(id: isize, place: Place) -> Result<String> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
    let hwnd = HWND(id as *mut _);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            bail!("that window is gone");
        }
        match place {
            Place::OnTop | Place::NotOnTop => {
                let after = if place == Place::OnTop { HWND_TOPMOST } else { HWND_NOTOPMOST };
                SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)?;
                return Ok(if place == Place::OnTop { "kept on top of the other windows".into() } else { "no longer kept on top".into() });
            }
            _ => {}
        }
        if IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            bail!("couldn't read its screen");
        }
        let r = info.rcWork;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (x, y, cw, ch) = match place {
            Place::Left => (r.left, r.top, w / 2, h),
            Place::Right => (r.left + w / 2, r.top, w - w / 2, h),
            Place::Top => (r.left, r.top, w, h / 2),
            _ => (r.left, r.top + h / 2, w, h - h / 2),
        };
        SetWindowPos(hwnd, None, x, y, cw, ch, SWP_NOZORDER | SWP_NOACTIVATE)?;
        Ok(format!("on the {} half of its screen", match place {
            Place::Left => "left",
            Place::Right => "right",
            Place::Top => "top",
            _ => "bottom",
        }))
    }
}
