//! Looking at windows and clicking at a point: a window's picture as its app draws it (`PrintWindow`, so whatever
//! covers it, her own overlay included, isn't in it), and a real mouse click with a check that the window meant is
//! the one under the pointer.

use anyhow::{Result, anyhow, bail};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, HDC, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};
use windows::core::BOOL;
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_MOUSE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, SendInput};
use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetSystemMetrics, GetWindowRect, IsIconic, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WindowFromPoint};

/// A window's picture: RGB pixels, top row first, and where it sits on screen.
pub struct Shot {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
}

/// The window's picture as its app draws it. A minimized window has none (bring it up first).
/// How much of a window looks different between two pictures of it (0..1, every fourth pixel each way): a click
/// that did something shows up here even when the title stays the same. Different sizes count as all changed.
pub fn changed(before: &Shot, after: &Shot) -> f32 {
    if before.width != after.width || before.height != after.height || before.rgb.len() != after.rgb.len() {
        return 1.0;
    }
    let (w, h) = (before.width as usize, before.height as usize);
    let (mut n, mut diff) = (0u32, 0u32);
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let i = (y * w + x) * 3;
            n += 1;
            if (0..3).any(|c| before.rgb[i + c].abs_diff(after.rgb[i + c]) > 24) {
                diff += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { diff as f32 / n as f32 }
}

pub fn capture(window: isize) -> Result<Shot> {
    let hwnd = HWND(window as *mut _);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            bail!("the window is minimized");
        }
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r)?;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 {
            bail!("the window has no size");
        }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, HGDIOBJ(bmp.0));
        // PW_RENDERFULLCONTENT: browsers, Electron and DirectX windows draw into it too.
        let drawn = PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(2)).as_bool();
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(mem, bmp, 0, h as u32, Some(bgra.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if !drawn || lines == 0 {
            bail!("the window wouldn't draw itself");
        }
        let rgb = bgra.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0]]).collect();
        Ok(Shot { rgb, width: w as u32, height: h as u32, left: r.left, top: r.top })
    }
}

/// The screen the window in front is on, as you see it but without her: Windows leaves our own windows out of the
/// capture for that moment (so she doesn't flicker), then they're back.
pub fn screen_shot() -> Result<Shot> {
    unsafe {
        let mut ours: Vec<HWND> = Vec::new();
        unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
            unsafe {
                let mut pid = 0;
                GetWindowThreadProcessId(h, Some(&mut pid));
                if pid == GetCurrentProcessId() && IsWindowVisible(h).as_bool() {
                    (*(l.0 as *mut Vec<HWND>)).push(h);
                }
            }
            BOOL(1)
        }
        let _ = EnumWindows(Some(each), LPARAM(&mut ours as *mut _ as isize));
        for h in &ours {
            let _ = SetWindowDisplayAffinity(*h, WDA_EXCLUDEFROMCAPTURE);
        }
        // A frame for the compositor to take them out.
        std::thread::sleep(std::time::Duration::from_millis(80));
        let shot = grab_monitor();
        for h in &ours {
            let _ = SetWindowDisplayAffinity(*h, WDA_NONE);
        }
        shot
    }
}

/// The pixels of the monitor the window in front is on.
unsafe fn grab_monitor() -> Result<Shot> {
    unsafe {
        let mon = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            bail!("couldn't find the screen");
        }
        let r = info.rcMonitor;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, HGDIOBJ(bmp.0));
        let copied = BitBlt(mem, 0, 0, w, h, Some(HDC(screen.0)), r.left, r.top, SRCCOPY | CAPTUREBLT).is_ok();
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(mem, bmp, 0, h as u32, Some(bgra.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if !copied || lines == 0 {
            bail!("the screen couldn't be copied");
        }
        let rgb = bgra.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0]]).collect();
        Ok(Shot { rgb, width: w as u32, height: h as u32, left: r.left, top: r.top })
    }
}

/// The top-level window under a point, as if her own windows weren't there. While your pointer is on her, her overlay
/// catches the mouse and was "the window under" every point, so sitting on a window from her menu found none.
pub fn window_at_not_mine(x: i32, y: i32) -> Option<isize> {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::{GW_HWNDNEXT, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, WS_EX_TRANSPARENT};
    unsafe {
        let me = GetCurrentProcessId();
        let pid_of = |h: HWND| {
            let mut p = 0u32;
            GetWindowThreadProcessId(h, Some(&mut p));
            p
        };
        let h = WindowFromPoint(POINT { x, y });
        if h.0.is_null() {
            return None;
        }
        let root = GetAncestor(h, GA_ROOT);
        let h = if root.0.is_null() { h } else { root };
        if pid_of(h) != me {
            return Some(h.0 as isize);
        }
        // Below hers, front to back: the first window that's shown, not see-through to the mouse, and over the point.
        let mut next = GetWindow(h, GW_HWNDNEXT).ok();
        while let Some(w) = next.filter(|w| !w.0.is_null()) {
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(w, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
            let ex = GetWindowLongPtrW(w, GWL_EXSTYLE) as u32;
            let mut r = RECT::default();
            let over = GetWindowRect(w, &mut r).is_ok() && x >= r.left && x < r.right && y >= r.top && y < r.bottom;
            if over && IsWindowVisible(w).as_bool() && !IsIconic(w).as_bool() && cloaked == 0 && ex & WS_EX_TRANSPARENT.0 == 0 && pid_of(w) != me {
                return Some(w.0 as isize);
            }
            next = GetWindow(w, GW_HWNDNEXT).ok();
        }
        None
    }
}

/// The top-level window under a point on screen.
pub fn window_at(x: i32, y: i32) -> Option<isize> {
    unsafe {
        let h = WindowFromPoint(POINT { x, y });
        if h.0.is_null() {
            return None;
        }
        let root = GetAncestor(h, GA_ROOT);
        Some(if root.0.is_null() { h.0 as isize } else { root.0 as isize })
    }
}

/// A real left click at a point on screen, for what can't be pressed any other way (her eyes found it; the control
/// has no button pattern). Your pointer isn't hers to keep: she waits until your hand is off the mouse, hides the
/// pointer, clicks, puts it back where it was and shows it again, all in about a twentieth of a second. Never over a
/// fullscreen game (it would lose the game its focus): that one waits for you to tab out.
pub fn click_point(x: i32, y: i32) -> Result<()> {
    let back = borrow_pointer(x, y)?;
    let _hidden = HiddenPointer::new();
    send_mouse(&[(x, y, MOUSEEVENTF_MOVE), (x, y, MOUSEEVENTF_LEFTDOWN), (x, y, MOUSEEVENTF_LEFTUP)])?;
    // The app takes the press with the pointer still over it (some check where it is on release).
    std::thread::sleep(std::time::Duration::from_millis(45));
    send_mouse(&[(back.0, back.1, MOUSEEVENTF_MOVE)])
}

/// A drag from one screen point to another (a slider, a scroll bar, a file onto a window), borrowing the pointer the
/// same way as a click: pressed at `from`, moved there in small steps (apps follow a drag, not a jump), let go at `to`,
/// and back where you had it.
pub fn drag_points(from: (i32, i32), to: (i32, i32)) -> Result<()> {
    let back = borrow_pointer(from.0, from.1)?;
    let _hidden = HiddenPointer::new();
    send_mouse(&[(from.0, from.1, MOUSEEVENTF_MOVE), (from.0, from.1, MOUSEEVENTF_LEFTDOWN)])?;
    let steps = 12;
    for i in 1..=steps {
        let (x, y) = (from.0 + (to.0 - from.0) * i / steps, from.1 + (to.1 - from.1) * i / steps);
        std::thread::sleep(std::time::Duration::from_millis(12));
        send_mouse(&[(x, y, MOUSEEVENTF_MOVE)])?;
    }
    std::thread::sleep(std::time::Duration::from_millis(40));
    send_mouse(&[(to.0, to.1, MOUSEEVENTF_LEFTUP)])?;
    std::thread::sleep(std::time::Duration::from_millis(45));
    send_mouse(&[(back.0, back.1, MOUSEEVENTF_MOVE)])
}

/// Waits until your hand is off the mouse (still for a third of a second, six seconds at most) and gives back where
/// your pointer is. Never over a fullscreen game (it would lose its focus).
fn borrow_pointer(x: i32, y: i32) -> Result<(i32, i32)> {
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    if super::gpu::front_fullscreen() && window_at(x, y) != Some(unsafe { GetForegroundWindow() }.0 as isize) {
        bail!("that needs a real click and a game is in front: it would lose its focus. I can do it when you tab out");
    }
    let pos = || {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p) }.map(|_| (p.x, p.y)).unwrap_or_default()
    };
    let (start, mut last, mut still) = (Instant::now(), pos(), Instant::now());
    while still.elapsed() < Duration::from_millis(350) {
        if start.elapsed() > Duration::from_secs(6) {
            bail!("your mouse kept moving; I need it still for a moment to click (or it's something only a real click reaches)");
        }
        std::thread::sleep(Duration::from_millis(40));
        let now = pos();
        if now != last {
            (last, still) = (now, Instant::now());
        }
    }
    Ok(last)
}

/// Mouse input at screen points, in one go.
fn send_mouse(events: &[(i32, i32, MOUSE_EVENT_FLAGS)]) -> Result<()> {
    unsafe {
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        let inputs: Vec<INPUT> = events
            .iter()
            .map(|&(x, y, flags)| {
                let (nx, ny) = ((x - vx) * 65535 / vw.max(1), (y - vy) * 65535 / vh.max(1));
                INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: nx, dy: ny, mouseData: 0, dwFlags: flags | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK, time: 0, dwExtraInfo: 0 } } }
            })
            .collect();
        if SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) as usize != inputs.len() {
            return Err(anyhow!("Windows blocked the click"));
        }
    }
    Ok(())
}

/// The pointer, invisible while it exists: every pointer shape swapped for an empty one, your own put back when it's
/// dropped (Windows reloads your pointer scheme). `restore_pointer` at her start undoes it if she ever died midway.
struct HiddenPointer;

impl HiddenPointer {
    fn new() -> Self {
        use windows::Win32::UI::WindowsAndMessaging::{CopyIcon, CreateCursor, HCURSOR, HICON, OCR_APPSTARTING, OCR_CROSS, OCR_HAND, OCR_IBEAM, OCR_NORMAL, OCR_SIZEALL, OCR_SIZENESW, OCR_SIZENS, OCR_SIZENWSE, OCR_SIZEWE, OCR_WAIT, SetSystemCursor};
        unsafe {
            let and = [0xFFu8; 128];
            let xor = [0u8; 128];
            if let Ok(blank) = CreateCursor(None, 0, 0, 32, 32, and.as_ptr().cast(), xor.as_ptr().cast()) {
                for id in [OCR_NORMAL, OCR_IBEAM, OCR_HAND, OCR_APPSTARTING, OCR_WAIT, OCR_CROSS, OCR_SIZEALL, OCR_SIZENESW, OCR_SIZENS, OCR_SIZENWSE, OCR_SIZEWE] {
                    // Each call takes the cursor it's given: a copy each.
                    if let Ok(copy) = CopyIcon(HICON(blank.0)) {
                        let _ = SetSystemCursor(HCURSOR(copy.0), id);
                    }
                }
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyCursor(blank);
            }
        }
        Self
    }
}

impl Drop for HiddenPointer {
    fn drop(&mut self) {
        restore_pointer();
    }
}

/// Your own pointer shapes back, from your settings.
pub fn restore_pointer() {
    use windows::Win32::UI::WindowsAndMessaging::{SPI_SETCURSORS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
    unsafe {
        let _ = SystemParametersInfoW(SPI_SETCURSORS, 0, None, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
}

/// Whether (x, y) is on window `id`'s own close, minimize, maximize or system-menu button (its title bar's), which a
/// click on its page must never press: the window says so itself (WM_NCHITTEST), and its top-right corner counts too
/// for windows that draw their own title bar. ("Click the X" on a page's popup closed the whole browser window.)
pub fn caption_button_at(id: isize, x: i32, y: i32) -> bool {
    use windows::Win32::Foundation::WPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_NCHITTEST};
    let hwnd = HWND(id as *mut _);
    let mut hit: usize = 0;
    let lp = LPARAM((((y as u32) & 0xffff) << 16 | ((x as u32) & 0xffff)) as isize);
    let asked = unsafe { SendMessageTimeoutW(hwnd, WM_NCHITTEST, WPARAM(0), lp, SMTO_ABORTIFHUNG, 300, Some(&mut hit)) };
    // HTSYSMENU 3, HTMINBUTTON 8, HTMAXBUTTON 9, HTCLOSE 20, HTHELP 21.
    if asked.0 != 0 && matches!(hit as u32 as i32, 3 | 8 | 9 | 20 | 21) {
        return true;
    }
    let mut r = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
        return false;
    }
    let scale = (unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96)) as f32 / 96.0;
    y >= r.top && y < r.top + (40.0 * scale) as i32 && x <= r.right && x > r.right - (150.0 * scale) as i32
}

#[cfg(test)]
mod tests {
    use super::{Shot, changed};

    fn shot(fill: u8) -> Shot {
        Shot { rgb: vec![fill; 40 * 40 * 3], width: 40, height: 40, left: 0, top: 0 }
    }

    #[test]
    fn a_click_that_did_nothing_changes_nothing() {
        assert_eq!(changed(&shot(10), &shot(10)), 0.0);
        let mut after = shot(10);
        // A menu opened over the top quarter.
        after.rgb[..40 * 10 * 3].fill(200);
        assert!(changed(&shot(10), &after) > 0.2);
        assert_eq!(changed(&shot(10), &Shot { width: 20, ..shot(10) }), 1.0);
    }
}
