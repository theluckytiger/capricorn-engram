//! Watching for games: every 2 s, the program in front and its share of the GPU. When a game starts her local model
//! leaves the GPU and she steps aside; the first time she sees a game she asks quiet or reachable and remembers it.
//! When it closes she comes back (`capri://game` to her overlay).

use std::time::Duration;

use capri_core::settings::GameMode;
use capri_hands::games::{Detector, Look};
use capri_hands::platform;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize)]
struct GameEvent {
    on: bool,
    exe: String,
    /// What her answer is remembered by (`games::family`): the game's install folder, or its exe.
    key: String,
    title: String,
    /// First time with this game: she asks quiet or reachable.
    ask: bool,
}

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("games".into())
        .spawn(move || {
            let mut meter = match platform::GpuMeter::new() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("game detection is off: {e:#}");
                    return;
                }
            };
            let me = std::process::id();
            let mut detector = Detector::default();
            let mut playing: Option<Look> = None;
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let gpu = match meter.sample() {
                    Ok(g) => g,
                    Err(e) => {
                        tracing::debug!("gpu: {e:#}");
                        continue;
                    }
                };
                if let Some(game) = &playing {
                    // A game lasts until it closes; alt-tabbing out of it doesn't end it.
                    if !platform::alive(game.pid) {
                        tracing::info!("game over: {}", game.exe);
                        set(&app, false, game);
                        playing = None;
                    }
                    continue;
                }
                let Some(w) = platform::list_windows().into_iter().find(|w| w.foreground) else { continue };
                if w.pid == me {
                    continue;
                }
                let look = Look {
                    pid: w.pid,
                    exe: w.exe.clone(),
                    path: platform::exe_path(w.pid).unwrap_or_default().to_lowercase(),
                    title: w.title.clone(),
                    fullscreen: platform::front_fullscreen(),
                    gpu: gpu.get(&w.pid).copied().unwrap_or(0.0),
                };
                if let Some(game) = detector.feed(look) {
                    tracing::info!("game: {} (\"{}\", {:.0}% GPU, fullscreen {})", game.exe, game.title, game.gpu, game.fullscreen);
                    set(&app, true, &game);
                    playing = Some(game);
                }
            }
        })
        .expect("games thread");
}

/// Her answer for this game: by its install folder (a launcher and its game share one), else by its exe (answers
/// saved before folders counted).
fn mode_of(app: &AppHandle, game: &Look) -> Option<GameMode> {
    let state = app.state::<crate::App>();
    let settings = state.settings.lock().unwrap();
    settings.games.get(&capri_hands::games::family(&game.exe, &game.path)).or_else(|| settings.games.get(&game.exe)).copied()
}

/// Quiet closes her ears for the game; anything else (or the game ending) puts them back as her settings say.
fn apply(app: &AppHandle, quiet: bool) {
    let l = app.state::<crate::listen::Listening>();
    if let Some(e) = &l.ears {
        e.set_always(!quiet && l.wake_word_on());
    }
}

/// The game on now (its window's title, and whether it's a quiet one), for her check-ins.
static NOW: std::sync::Mutex<Option<(String, bool)>> = std::sync::Mutex::new(None);

pub fn now() -> Option<(String, bool)> {
    NOW.lock().unwrap().clone()
}

fn set(app: &AppHandle, on: bool, game: &Look) {
    *NOW.lock().unwrap() = on.then(|| (game.title.clone(), mode_of(app, game) == Some(GameMode::Quiet)));
    let brain = app.state::<crate::brain::Mind>().brain();
    tauri::async_runtime::spawn(async move { brain.set_gaming(on).await });
    let mode = mode_of(app, game);
    let key = capri_hands::games::family(&game.exe, &game.path);
    capri_core::journal::write("game", serde_json::json!({ "on": on, "exe": game.exe, "key": key, "title": game.title, "gpu": game.gpu, "fullscreen": game.fullscreen, "mode": mode }));
    apply(app, on && mode == Some(GameMode::Quiet));
    let _ = app.emit("capri://game", GameEvent { on, exe: game.exe.clone(), key, title: game.title.clone(), ask: on && mode.is_none() });
}

/// Her question answered: remember it for this game, and do it now.
#[tauri::command]
pub fn game_mode(app: AppHandle, exe: String, quiet: bool) -> Result<(), String> {
    let mode = if quiet { GameMode::Quiet } else { GameMode::Reachable };
    let state = app.state::<crate::App>();
    let mut settings = state.settings.lock().unwrap();
    settings.games.insert(exe.clone(), mode);
    // The file may have changed since start (set_home writes it): merge into what's there.
    let mut on_disk = capri_core::Settings::load().unwrap_or_else(|_| settings.clone());
    on_disk.games.insert(exe.clone(), mode);
    on_disk.save().map_err(|e| e.to_string())?;
    drop(settings);
    tracing::info!("{exe}: {mode:?}");
    capri_core::journal::write("game_mode", serde_json::json!({ "exe": exe, "mode": mode }));
    apply(&app, quiet);
    Ok(())
}
