//! Her overlay against the taskbar: puts the taskbar at the top of the always-on-top band (what a click on it does),
//! then says, a moment later, whether her overlay is above it again. `cargo run -p capri-hands --example on_top`
#[cfg(windows)]
fn main() {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::w;
    unsafe {
        let bar = FindWindowW(w!("Shell_TrayWnd"), None).expect("no taskbar");
        let order = || {
            // Top to bottom: where the taskbar and her overlays are.
            let (mut h, mut i, mut out) = (GetTopWindow(None).unwrap_or_default(), 0, Vec::new());
            while !h.0.is_null() && i < 400 {
                let mut class = [0u16; 64];
                let n = GetClassNameW(h, &mut class) as usize;
                let mut title = [0u16; 64];
                let t = GetWindowTextW(h, &mut title) as usize;
                let (class, title) = (String::from_utf16_lossy(&class[..n]), String::from_utf16_lossy(&title[..t]));
                if h == bar || (title == "Capri" && IsWindowVisible(h).as_bool()) {
                    out.push(format!("{i}:{}", if h == bar { "taskbar".to_string() } else { format!("Capri ({class})") }));
                }
                h = GetWindow(h, GW_HWNDNEXT).unwrap_or_default();
                i += 1;
            }
            out.join("  ")
        };
        println!("before:        {}", order());
        let _ = SetWindowPos(bar, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        println!("taskbar raised: {}", order());
        std::thread::sleep(std::time::Duration::from_millis(700));
        println!("0.7 s later:   {}", order());
    }
}

#[cfg(not(windows))]
fn main() {}
