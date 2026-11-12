//! Pointing something out on your screen ("circle the start button", "where's the download button?", "show me the
//! mute switch"): she finds it (by its name among the window's controls, else with her own eyes), and her overlay
//! points at it, a wire from her hand to it and its name on it. Nothing is clicked.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::controls::pick;
use crate::eyes::find;
use crate::pc::find_window;
use crate::platform;
use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

pub struct PointOut;

#[async_trait]
impl Tool for PointOut {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "point_out".into(),
            description: "Show the user where something is on their screen, without clicking it (\"circle the start button\", \
                          \"where's the download button?\", \"point at the mute switch\"): she points at it and marks it. \
                          `window`: the app's name, words of its title, or \"front\"; `target`: the thing, as it looks."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "window": { "type": "string" }, "target": { "type": "string" } },
                "required": ["target"]
            }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Read
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let target: String = match arg(&args, "target") {
            Ok(t) => t,
            Err(o) => return o,
        };
        let name: String = opt_arg(&args, "window").unwrap_or_else(|| "front".into());
        // Windows' own Start button and taskbar are a window too.
        let name = if ["start", "taskbar", "start button", "start menu"].iter().any(|w| target.to_lowercase().contains(w)) && name == "front" { "taskbar".to_string() } else { name };
        let wins: Vec<_> = platform::list_windows().into_iter().filter(|w| w.pid != ctx.own_pid).collect();
        let w = if name == "taskbar" {
            match taskbar() {
                Some(w) => w,
                None => return Outcome::failed("couldn't find the taskbar"),
            }
        } else {
            match find_window(&name, &wins) {
                Ok(w) => w,
                Err(o) => return o,
            }
        };
        // By name first: exact, quick, and no picture needed.
        let (id, t) = (w.id, target.clone());
        let by_name = tokio::task::spawn_blocking(move || {
            let cs = platform::controls(id, 1500).unwrap_or_default();
            pick(&t, None, &cs).ok().map(|c| (c.rect, c.name.clone()))
        })
        .await
        .ok()
        .flatten();
        let (rect, label) = match by_name {
            Some(found) => found,
            None => {
                let id = w.id;
                let shot = match tokio::task::spawn_blocking(move || platform::capture(id).map(|s| { let l = platform::ocr::read(&s).unwrap_or_default(); (s, l) })).await {
                    Ok(Ok(s)) => s,
                    _ => return Outcome::failed(format!("couldn't take a picture of {}", w.app)),
                };
                let (shot, labels) = shot;
                if let Some(l) = crate::eyes::by_words(&labels, &target, None) {
                    let point = l.center();
                    return Outcome::ok(format!("pointed out \"{}\" in {} (at {}, {})", l.text, w.app, point.0, point.1)).with(json!({ "point": [point.0, point.1], "rect": l.rect, "label": l.text }));
                }
                let Some(eyes) = &ctx.eyes else { return Outcome::failed(format!("nothing called \"{target}\" among {}'s controls or the words on it, and her eyes are off", w.app)) };
                match find(eyes.0.as_ref(), &shot.rgb, shot.width, shot.height, &target).await {
                    Ok(f) => {
                        let (x, y) = (shot.left + f.x, shot.top + f.y);
                        ([x - 30, y - 20, x + 30, y + 20], if f.label.is_empty() { target.clone() } else { f.label })
                    }
                    Err(why) => return Outcome::failed(format!("I looked at {} for \"{target}\": {why}", w.app)),
                }
            }
        };
        let point = [(rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2];
        Outcome::ok(format!("pointed out \"{label}\" in {} (at {}, {})", w.app, point[0], point[1])).with(json!({ "point": point, "rect": rect, "label": label }))
    }
}

/// The taskbar (the Start button is on it), as a window.
fn taskbar() -> Option<platform::WinInfo> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect};
    use windows::core::w;
    unsafe {
        let h = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        Some(platform::WinInfo { id: h.0 as isize, title: "Taskbar".into(), app: "Windows".into(), exe: "explorer".into(), pid: 0, rect: [r.left, r.top, r.right, r.bottom], minimized: false, maximized: false, foreground: false })
    }
}
