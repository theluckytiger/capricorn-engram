//! Her overlay windows: one transparent, always-on-top window per monitor. The mouse goes through them everywhere
//! except on her: the renderer sends her outline (`set_hull`) and a loop here flips click-through on and off as the
//! pointer enters and leaves it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

/// She's riding your pointer (Ctrl+Alt+R): where it is goes to her overlays as `capri://cursor`, 30 times a second.
static RIDING: AtomicBool = AtomicBool::new(false);

/// Each overlay window's handle (to hide it from any thread) and when its page last said it's alive.
static WATCHED: Mutex<Vec<(String, isize, Option<std::time::Instant>)>> = Mutex::new(Vec::new());

/// Dev builds only: freezes her main thread for `secs`, to see the watchdog hide her overlays and bring them back.
#[tauri::command]
pub fn hang_test(secs: u64) {
    if cfg!(debug_assertions) {
        std::thread::sleep(Duration::from_secs(secs.min(10)));
    }
}

/// Her page says it's alive (every two seconds).
#[tauri::command]
pub fn alive(window: tauri::WebviewWindow) {
    if let Some(w) = WATCHED.lock().unwrap().iter_mut().find(|w| w.0 == window.label()) {
        w.2 = Some(std::time::Instant::now());
    }
}

/// Her overlays cover whole screens, always on top. If her main thread stops answering (a hang) or a page stops
/// saying it's alive (it crashed, or failed to load and shows a white error page), that screen's overlay is hidden at
/// once from this thread, without waiting on the stuck one, so your screens stay usable; it's shown again when
/// things answer again.
pub fn start_watchdog(app: AppHandle) {
    std::thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let answered = std::sync::Arc::new(AtomicBool::new(true));
            let mut hidden: Vec<String> = Vec::new();
            let mut hung_since: Option<std::time::Instant> = None;
            // A dead page, hidden, gets reloaded (once every 30 s at most) so it can come back: hidden, it would only
            // be throttled harder and never answer again.
            let mut reloaded: HashMap<String, std::time::Instant> = HashMap::new();
            loop {
                std::thread::sleep(Duration::from_millis(500));
                // The main thread: one ping at a time; unanswered for two seconds is a hang.
                if answered.swap(false, Ordering::Relaxed) {
                    hung_since = None;
                    let a = answered.clone();
                    let _ = app.run_on_main_thread(move || a.store(true, Ordering::Relaxed));
                } else {
                    hung_since.get_or_insert_with(std::time::Instant::now);
                }
                let hung = hung_since.is_some_and(|t| t.elapsed() > Duration::from_secs(2));
                let watched = WATCHED.lock().unwrap().clone();
                for (label, hwnd, beat) in watched {
                    // A page gets 20 s to load at first; after that, 8 s without a word means it's dead.
                    let dead = match beat {
                        Some(t) => t.elapsed() > Duration::from_secs(8),
                        None => started.elapsed() > Duration::from_secs(20),
                    };
                    let bad = hung || dead;
                    let is_hidden = hidden.contains(&label);
                    if bad && !is_hidden {
                        #[cfg(windows)]
                        capri_hands::platform::set_shown(hwnd, false);
                        tracing::error!("{label}: {} — hidden so it can't cover the screen", if hung { "her main thread stopped answering" } else { "its page stopped answering" });
                        capri_core::journal::write("watchdog", serde_json::json!({ "overlay": label, "hidden": true, "why": if hung { "main thread hung" } else { "page dead" } }));
                        hidden.push(label);
                    } else if bad && is_hidden && !hung && dead && reloaded.get(&label).is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                        reloaded.insert(label.clone(), std::time::Instant::now());
                        if let Some(w) = app.get_webview_window(&label) {
                            let r = w.reload();
                            tracing::warn!("{label}: still not answering, reloaded ({r:?})");
                            capri_core::journal::write("watchdog", serde_json::json!({ "overlay": label, "reloaded": true }));
                        }
                    } else if !bad && is_hidden {
                        #[cfg(windows)]
                        capri_hands::platform::set_shown(hwnd, true);
                        tracing::info!("{label}: answering again, shown");
                        capri_core::journal::write("watchdog", serde_json::json!({ "overlay": label, "hidden": false }));
                        hidden.retain(|l| *l != label);
                    }
                }
            }
        })
        .expect("watchdog thread");
}

/// The overlay she's on (to follow your pointer to another screen).
static HERE: Mutex<String> = Mutex::new(String::new());

/// She's gone from this screen to the one your pointer is on: that overlay takes her (`capri://come`, where the
/// pointer is), with what she carries (her panel, the ticker over her head).
#[tauri::command]
pub fn follow_to(app: AppHandle, window: tauri::WebviewWindow, to: String, x: f64, y: f64, carry: Option<serde_json::Value>) {
    capri_core::journal::write("screen", serde_json::json!({ "from": window.label(), "to": to, "why": "following the pointer" }));
    *HERE.lock().unwrap() = to.clone();
    let _ = tauri::Emitter::emit_to(&app, to.as_str(), "capri://come", serde_json::json!({ "x": x, "y": y, "carry": carry }));
}

/// Where your pointer is (physical px of the whole desktop): her page only sees it while it's over her.
#[tauri::command]
pub fn pointer_at(app: AppHandle) -> Option<(f64, f64)> {
    app.cursor_position().ok().map(|c| (c.x, c.y))
}

/// "Go to my other screen": tell the overlay she's on to teleport to the next monitor (vanish → come).
pub fn go_other_screen(app: &AppHandle) -> bool {
    let here = HERE.lock().unwrap().clone();
    let Ok(monitors) = app.available_monitors() else { return false };
    if monitors.len() < 2 {
        return false;
    }
    let here_i = here.strip_prefix("overlay-").and_then(|i| i.parse::<usize>().ok()).unwrap_or(0);
    let other_i = (0..monitors.len()).find(|&i| i != here_i).unwrap_or(0);
    let m = &monitors[other_i];
    let scale = m.scale_factor();
    let to = format!("overlay-{other_i}");
    let x = m.size().width as f64 / scale / 2.0;
    let y = (m.size().height as f64 / scale) * 0.72;
    capri_core::journal::write("screen", serde_json::json!({ "from": here, "to": to, "why": "asked" }));
    tauri::Emitter::emit_to(app, here.as_str(), "capri://go_other", serde_json::json!({ "to": to, "x": x, "y": y })).is_ok()
}

/// Which screen she's on (a page that reloads asks, instead of assuming the first screen).
#[tauri::command]
pub fn whereabouts() -> String {
    HERE.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_riding(on: bool) {
    RIDING.store(on, Ordering::Relaxed);
}

#[derive(Default)]
pub struct Overlays {
    /// Per overlay window: her outline in that window's physical pixels, and whether the pointer is held on her.
    state: Mutex<HashMap<String, Hit>>,
}

#[derive(Default, Clone)]
struct Hit {
    polygons: Vec<Vec<(f64, f64)>>,
    /// The window's top-left on screen, physical pixels (asking the window each tick would go through the main thread).
    origin: Option<(f64, f64)>,
    grabbed: bool,
    /// What we last told the window, so we only call into the OS when it changes.
    catching: bool,
}

pub fn open_all(app: &AppHandle) -> tauri::Result<()> {
    let monitors = app.available_monitors()?;
    *HERE.lock().unwrap() = "overlay-0".into();
    // Showing a window activates it: she took the keyboard from whatever you were in every time she started.
    #[cfg(windows)]
    let was = capri_hands::platform::foreground();
    for (i, m) in monitors.iter().enumerate() {
        let label = format!("overlay-{i}");
        let pos = *m.position();
        let size = *m.size();
        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(format!("overlay.html?screen={i}").into()))
            .title("Capri")
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .focused(false)
            // Never throttled: a see-through overlay can look unseen to WebView2, which then slowed her heartbeat
            // until the watchdog hid the screen for good (she vanished from screen 2 on 2026-11-03).
            .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
            .visible(false)
            .build()?;
        window.set_position(PhysicalPosition::new(pos.x, pos.y))?;
        // One pixel short of the full height, so Windows never mistakes her for a fullscreen app (that would hide
        // the taskbar and turn on "do not disturb").
        window.set_size(PhysicalSize::new(size.width, size.height.saturating_sub(1)))?;
        window.set_ignore_cursor_events(true)?;
        window.show()?;
        #[cfg(windows)]
        if let Ok(h) = window.hwnd() {
            WATCHED.lock().unwrap().push((label.clone(), h.0 as isize, None));
        }
        tracing::info!("{label}: {}x{} at {},{} (scale {})", size.width, size.height, pos.x, pos.y, m.scale_factor());
    }
    #[cfg(windows)]
    capri_hands::platform::give_back_focus(was);
    Ok(())
}

#[derive(Deserialize)]
pub struct HullArgs {
    /// Polygons in CSS pixels of the window that sent them.
    polygons: Vec<Vec<[f64; 2]>>,
}

// Both commands run off the main thread (`async`): the main thread must stay free for click-through flips.
#[tauri::command(async)]
pub fn set_hull(window: tauri::WebviewWindow, state: tauri::State<'_, crate::App>, args: HullArgs) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let origin = window.outer_position().ok().map(|p| (p.x as f64, p.y as f64));
    let polygons: Vec<Vec<(f64, f64)>> = args.polygons.into_iter().map(|p| p.into_iter().map(|[x, y]| (x * scale, y * scale)).collect()).collect();
    let mut map = state.overlays.state.lock().unwrap();
    let hit = map.entry(window.label().to_string()).or_default();
    if hit.polygons.is_empty() {
        tracing::debug!("{}: first outline, {} polygons", window.label(), polygons.len());
    }
    hit.polygons = polygons;
    hit.origin = origin;
}

/// Another screen, in this overlay's CSS px (where she'd fly into it).
#[derive(serde::Serialize)]
pub struct Around {
    label: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Where her floor is on this screen, in its CSS px: the top of the taskbar (the bottom of the screen's work area;
/// the screen's bottom when the taskbar hides itself).
#[tauri::command(async)]
pub fn taskbar_top(window: tauri::WebviewWindow) -> Option<f64> {
    let (scale, origin, m) = (window.scale_factor().ok()?, window.outer_position().ok()?, window.current_monitor().ok()??);
    let wa = m.work_area();
    Some((wa.position.y + wa.size.height as i32 - origin.y) as f64 / scale)
}

/// The other screens, around this one.
#[tauri::command(async)]
pub fn screens_around(app: AppHandle, window: tauri::WebviewWindow) -> Vec<Around> {
    let (Ok(scale), Ok(origin), Ok(monitors)) = (window.scale_factor(), window.outer_position(), app.available_monitors()) else { return vec![] };
    monitors
        .iter()
        .enumerate()
        .map(|(i, m)| (format!("overlay-{i}"), m))
        .filter(|(label, _)| label != window.label())
        .map(|(label, m)| Around { label, x: (m.position().x - origin.x) as f64 / scale, y: (m.position().y - origin.y) as f64 / scale, w: m.size().width as f64 / scale, h: m.size().height as f64 / scale })
        .collect()
}

#[derive(Deserialize)]
pub struct FlyArgs {
    to: String,
    /// Where she is and how she's flying, in the sending window's CSS px (her middle, just past its edge).
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    /// Her spin: the angle she's at, and how fast it turns (degrees, per second).
    a: f64,
    w: f64,
    /// What goes with her: her panel, the ticker over her head.
    #[serde(default)]
    carry: Option<serde_json::Value>,
}

#[derive(Clone, serde::Serialize)]
struct FlyIn {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    a: f64,
    w: f64,
    carry: Option<serde_json::Value>,
}

/// Thrown past the edge of her screen into another one: that screen's overlay takes her, mid-flight.
#[tauri::command(async)]
pub fn fly_to(app: AppHandle, window: tauri::WebviewWindow, args: FlyArgs) -> bool {
    let (Ok(scale), Ok(origin), Ok(monitors)) = (window.scale_factor(), window.outer_position(), app.available_monitors()) else { return false };
    let Some(m) = args.to.strip_prefix("overlay-").and_then(|i| i.parse::<usize>().ok()).and_then(|i| monitors.get(i)) else { return false };
    let (p, ms) = (m.position(), m.scale_factor());
    let (gx, gy) = (origin.x as f64 + args.x * scale, origin.y as f64 + args.y * scale);
    // The same speed on screen, in that screen's CSS px.
    let k = scale / ms;
    let fly = FlyIn { x: (gx - p.x as f64) / ms, y: (gy - p.y as f64) / ms, vx: args.vx * k, vy: args.vy * k, a: args.a, w: args.w, carry: args.carry };
    capri_core::journal::write("screen", serde_json::json!({ "from": window.label(), "to": args.to, "why": "thrown" }));
    *HERE.lock().unwrap() = args.to.clone();
    tauri::Emitter::emit_to(&app, args.to.as_str(), "capri://fly_in", fly).is_ok()
}

/// A window she could sit on, in this overlay's CSS px: its top edge (`x`, `y`), how wide it is, and the part of that
/// edge in sight (`vx`, `vw`: where she can sit; the rest is under another window).
#[derive(serde::Serialize)]
pub struct Perch {
    id: isize,
    app: String,
    x: f64,
    y: f64,
    w: f64,
    /// Its height (for possessing it).
    h: f64,
    vx: f64,
    vw: f64,
}

/// The windows she could sit on, on this overlay's screen, front to back: shown (not minimized or maximized), wide
/// enough, with their top edge on this screen and not right at its top, and a good stretch of that edge really in
/// sight (Windows says the window under it is that one). Only the stretch in sight counts: the Claude window in front
/// covered the middle of every window's top edge behind it, and a window needed its edge clear at three fixed points
/// and 120 px down, so in an ordinary layout she had nowhere to sit and did nothing.
#[tauri::command(async)]
pub fn perches(window: tauri::WebviewWindow) -> Vec<Perch> {
    let (Ok(scale), Ok(origin), Ok(size)) = (window.scale_factor(), window.outer_position(), window.outer_size()) else { return vec![] };
    let (sw, sh) = (size.width as f64 / scale, size.height as f64 / scale);
    let me = std::process::id();
    capri_hands::platform::list_windows()
        .into_iter()
        .filter(|w| w.pid != me && !w.minimized && !w.maximized)
        .filter_map(|w| {
            let [l, t, r, b] = w.rect;
            let (x, y, wd) = ((l - origin.x) as f64 / scale, (t - origin.y) as f64 / scale, (r - l) as f64 / scale);
            if !(wd > 320.0 && x > -wd * 0.5 && x + wd * 0.5 < sw && y > 40.0 && y < sh - 120.0) {
                return None;
            }
            // Its top edge, looked at in 40 places: the longest stretch that's really it.
            const N: i32 = 40;
            let (mut best, mut run) = ((0, 0), (0, 0));
            for i in 0..N {
                let px = l + (r - l) * (2 * i + 1) / (2 * N);
                if capri_hands::platform::window_at_not_mine(px, t + 8) == Some(w.id) {
                    run = if run.1 == 0 { (i, 1) } else { (run.0, run.1 + 1) };
                    if run.1 > best.1 {
                        best = run;
                    }
                } else {
                    run = (0, 0);
                }
            }
            let (mut vx, mut vw) = (x + wd * best.0 as f64 / N as f64, wd * best.1 as f64 / N as f64);
            // Only the stretch that is on this screen: off-screen bins still return the HWND under the mouse, and she
            // used to sit on a ledge that hung off the edge (or under another monitor).
            const MARGIN: f64 = 40.0;
            let right = (vx + vw).min(sw - MARGIN);
            vx = vx.max(MARGIN);
            vw = right - vx;
            (vw >= 160.0).then_some(Perch { id: w.id, app: w.app, x, y, w: wd, h: (b - t) as f64 / scale, vx, vw })
        })
        .collect()
}

/// The window she'd possess: the one in front on this screen (any real window, maximized too), its middle in sight.
#[tauri::command(async)]
pub fn possessable(window: tauri::WebviewWindow) -> Option<Perch> {
    let (Ok(scale), Ok(origin), Ok(size)) = (window.scale_factor(), window.outer_position(), window.outer_size()) else { return None };
    let (sw, sh) = (size.width as f64 / scale, size.height as f64 / scale);
    let me = std::process::id();
    capri_hands::platform::list_windows().into_iter().filter(|w| w.pid != me && !w.minimized && w.title != "Program Manager").find_map(|w| {
        let [l, t, r, b] = w.rect;
        let (x, y, wd, h) = ((l - origin.x) as f64 / scale, (t - origin.y) as f64 / scale, (r - l) as f64 / scale, (b - t) as f64 / scale);
        let (cx, cy) = (x + wd / 2.0, y + h / 2.0);
        let mine = cx > 0.0 && cx < sw && cy > 0.0 && cy < sh;
        (mine && wd >= 420.0 && h >= 260.0 && capri_hands::platform::window_at_not_mine((l + r) / 2, (t + b) / 2) == Some(w.id))
            .then_some(Perch { id: w.id, app: w.app, x, y, w: wd, h, vx: x, vw: wd })
    })
}

/// She hit a window's top edge: it shakes (`power` 0..1).
#[tauri::command(async)]
pub fn shake_window(id: isize, power: f64) {
    #[cfg(windows)]
    capri_hands::platform::shake(id, power);
}

/// While you hold her (dragging), her window keeps the mouse even when the pointer leaves her outline.
#[tauri::command(async)]
pub fn set_grab(window: tauri::WebviewWindow, state: tauri::State<'_, crate::App>, on: bool) {
    let mut map = state.overlays.state.lock().unwrap();
    map.entry(window.label().to_string()).or_default().grabbed = on;
}

/// A screen: its overlay's label, where it is and its size (physical px), its scale.
type Screen = (String, (f64, f64, f64, f64), f64);

/// Your text cursor: each time it moves (you're typing, or clicked into a text box), where it is goes to her overlays
/// as `capri://caret` (left, top, right, bottom, physical px), so she can get out of the way.
pub fn watch_caret(app: AppHandle) {
    #[cfg(windows)]
    std::thread::Builder::new()
        .name("caret".into())
        .spawn(move || {
            let mut last: Option<[i32; 4]> = None;
            loop {
                std::thread::sleep(Duration::from_millis(150));
                let now = capri_hands::platform::caret();
                if now.is_some() && now != last {
                    let _ = app.emit("capri://caret", now);
                }
                last = now;
            }
        })
        .ok();
}

pub fn start_hit_test(app: AppHandle) {
    std::thread::Builder::new()
        .name("hit-test".into())
        .spawn(move || {
            let mut n = 0u64;
            // Following your pointer: the screens (looked up now and then), since when the pointer's been on
            // another screen than hers, and when she may go again.
            let mut screens: Vec<Screen> = Vec::new();
            let mut away_since: Option<std::time::Instant> = None;
            let mut calm_until = std::time::Instant::now();
            loop {
                std::thread::sleep(Duration::from_millis(16));
                let Ok(cursor) = app.cursor_position() else { continue };
                n += 1;
                if RIDING.load(Ordering::Relaxed) && n.is_multiple_of(2) {
                    let _ = app.emit("capri://cursor", (cursor.x, cursor.y));
                }
                // Four times a second: a click on the taskbar brought it over her (sitting on it): she's back on top.
                #[cfg(windows)]
                if n.is_multiple_of(15) {
                    let ids: Vec<isize> = WATCHED.lock().unwrap().iter().map(|w| w.1).collect();
                    for id in ids {
                        capri_hands::platform::above_taskbar(id);
                    }
                }
                // Four times a second: is the pointer on another screen than hers, five seconds now? She follows.
                if n.is_multiple_of(15) {
                    if screens.is_empty() || n.is_multiple_of(600) {
                        screens = app.available_monitors().unwrap_or_default().iter().enumerate().map(|(i, m)| (format!("overlay-{i}"), (m.position().x as f64, m.position().y as f64, m.size().width as f64, m.size().height as f64), m.scale_factor())).collect();
                    }
                    let follow = app.state::<crate::App>().settings.lock().unwrap().screens.follow_pointer;
                    let on = screens.iter().find(|(_, (x, y, w, h), _)| cursor.x >= *x && cursor.y >= *y && cursor.x < x + w && cursor.y < y + h);
                    let here = HERE.lock().unwrap().clone();
                    match on {
                        Some((label, (x, y, _, _), scale)) if *label != here && follow && !RIDING.load(Ordering::Relaxed) && !app.state::<crate::brain::Mind>().brain().is_gaming() => {
                            let since = *away_since.get_or_insert_with(std::time::Instant::now);
                            if since.elapsed() > Duration::from_secs(5) && std::time::Instant::now() > calm_until {
                                away_since = None;
                                calm_until = std::time::Instant::now() + Duration::from_secs(10);
                                let _ = tauri::Emitter::emit_to(&app, here.as_str(), "capri://follow", serde_json::json!({ "to": label, "x": (cursor.x - x) / scale, "y": (cursor.y - y) / scale }));
                            }
                        }
                        _ => away_since = None,
                    }
                }
                let state = app.state::<crate::App>();
                // Decide under the lock, act after releasing it: flipping click-through waits on the main thread, and
                // the main thread may be waiting on this lock (a new outline arriving) — holding it here would deadlock.
                let mut flips = Vec::new();
                {
                    let mut map = state.overlays.state.lock().unwrap();
                    for (label, hit) in map.iter_mut() {
                        let Some(origin) = hit.origin else { continue };
                        let p = (cursor.x - origin.0, cursor.y - origin.1);
                        let inside = hit.grabbed || hit.polygons.iter().any(|poly| contains(poly, p));
                        if inside != hit.catching {
                            hit.catching = inside;
                            flips.push((label.clone(), inside));
                        }
                    }
                }
                // On Windows, flipped straight on the window: never activating it, so you keep typing where you were.
                #[cfg(windows)]
                {
                    let windows = WATCHED.lock().unwrap().iter().map(|w| (w.0.clone(), w.1)).collect::<Vec<_>>();
                    for (label, inside) in flips {
                        if let Some((_, id)) = windows.iter().find(|w| w.0 == label) {
                            capri_hands::platform::set_click_through(*id, !inside);
                            tracing::debug!("{label}: {}", if inside { "catching" } else { "click-through" });
                        }
                    }
                    // Anything else that set the window's styles (the window library re-applying its own) is undone.
                    if n.is_multiple_of(15) {
                        let catching: Vec<bool> = {
                            let map = state.overlays.state.lock().unwrap();
                            windows.iter().map(|(label, _)| map.get(label).is_some_and(|h| h.catching)).collect()
                        };
                        for ((_, id), catching) in windows.iter().zip(catching) {
                            if capri_hands::platform::click_through(*id) == catching {
                                capri_hands::platform::set_click_through(*id, !catching);
                            }
                        }
                    }
                }
                #[cfg(not(windows))]
                for (label, inside) in flips {
                    if let Some(window) = app.get_webview_window(&label) {
                        let r = window.set_ignore_cursor_events(!inside);
                        tracing::debug!("{label}: {} ({r:?})", if inside { "catching" } else { "click-through" });
                    }
                }
            }
        })
        .expect("hit-test thread");
}

/// Even-odd point-in-polygon.
fn contains(poly: &[(f64, f64)], (x, y): (f64, f64)) -> bool {
    let mut inside = false;
    let mut j = poly.len().wrapping_sub(1);
    for i in 0..poly.len() {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::contains;

    #[test]
    fn square() {
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(contains(&sq, (5.0, 5.0)));
        assert!(!contains(&sq, (15.0, 5.0)));
        assert!(!contains(&[], (0.0, 0.0)));
    }
}
