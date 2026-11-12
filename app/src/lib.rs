//! The app: her overlay windows (one per monitor), the tray, hotkeys, and the bridge between the UI and her core.

mod brain;
mod care;
mod watch;
#[cfg(windows)]
mod feel;
mod games;
mod hooks;
mod listen;
mod models;
mod overlay;
mod phone;
mod prefs;
mod speak;
mod testlog;
mod tray;

use std::sync::Mutex;

use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Everything the app shares between windows and threads. Tauri hands it to commands as `State<App>`.
pub struct App {
    pub settings: Mutex<capri_core::Settings>,
    pub overlays: overlay::Overlays,
}

/// Her keys: Ctrl+Alt+letter, each one rebindable (Settings › Keys).
/// V: hold to talk, tap to have her listen. W: listening for her name on/off. T: type to her. C: put her away.
/// N: your mic muted for her, or back. R: she rides your pointer, or gets off.
const KEYS: &[(char, &str)] = &[('V', "talk"), ('W', "wake_word"), ('T', "type"), ('C', "away"), ('N', "mute"), ('R', "ride")];

/// The keys bound now: each letter's code, and what it does.
static BOUND: Mutex<Vec<(Code, &'static str)>> = Mutex::new(Vec::new());

fn code(letter: char) -> Option<Code> {
    const CODES: [Code; 26] = [Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD, Code::KeyE, Code::KeyF, Code::KeyG, Code::KeyH, Code::KeyI, Code::KeyJ, Code::KeyK, Code::KeyL, Code::KeyM, Code::KeyN, Code::KeyO, Code::KeyP, Code::KeyQ, Code::KeyR, Code::KeyS, Code::KeyT, Code::KeyU, Code::KeyV, Code::KeyW, Code::KeyX, Code::KeyY, Code::KeyZ];
    let c = letter.to_ascii_uppercase();
    c.is_ascii_uppercase().then(|| CODES[(c as u8 - b'A') as usize])
}

/// (Re)binds her keys as her settings say: the old ones let go, the new ones taken. Says which couldn't be taken.
pub fn bind_keys(app: &tauri::AppHandle, settings: &capri_core::Settings) -> Vec<String> {
    let mods = Some(Modifiers::CONTROL | Modifiers::ALT);
    let mut bound = BOUND.lock().unwrap();
    for (c, _) in bound.drain(..) {
        let _ = app.global_shortcut().unregister(Shortcut::new(mods, c));
    }
    let mut taken = Vec::new();
    for (usual, what) in KEYS {
        let letter = settings.keys.get(*what).copied().unwrap_or(*usual);
        let Some(c) = code(letter).or_else(|| code(*usual)) else { continue };
        if bound.iter().any(|(b, _)| *b == c) {
            taken.push(format!("Ctrl+Alt+{letter} is already one of hers"));
            continue;
        }
        match app.global_shortcut().register(Shortcut::new(mods, c)) {
            Ok(()) => bound.push((c, what)),
            Err(e) => {
                tracing::warn!("Ctrl+Alt+{letter} ({what}) is taken by another app: {e}");
                taken.push(format!("Ctrl+Alt+{letter} is taken by another app"));
            }
        }
    }
    taken
}

/// The beat her ear hears on the speakers now (a page that reloads asks; the event only comes when it changes).
static BEAT: std::sync::Mutex<Option<capri_ears::beat::Beat>> = std::sync::Mutex::new(None);

#[tauri::command]
fn beat_now() -> Option<capri_ears::beat::Beat> {
    *BEAT.lock().unwrap()
}

pub fn run() {
    logging();
    testlog::catch_panics();
    #[cfg(windows)]
    capri_core::journal::catch_crashes();
    capri_core::journal::write("start", serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "dev": cfg!(debug_assertions), "everything": capri_core::journal::everything() }));

    let mut settings = capri_core::Settings::load().unwrap_or_else(|e| {
        tracing::error!("settings: {e:#}");
        capri_core::Settings::default()
    });
    testlog::session(&settings);
    // Her local model, picked for this PC (unless you chose one): before her brain starts and before it's fetched.
    if settings.brain.auto {
        let (model, why) = models::recommended();
        capri_core::journal::write("setup", serde_json::json!({ "recommended": model, "why": why }));
        if settings.brain.fast != model && settings.brain.fast.starts_with("ollama/") {
            settings.brain.fast = model.clone();
            if settings.brain.eyes.as_deref().is_some_and(|e| e.starts_with("ollama/")) {
                settings.brain.eyes = Some(model);
            }
            let _ = settings.save();
        }
    }
    let ears_settings = settings.clone();
    let mind = brain::Mind::new(&settings).unwrap_or_else(|e| {
        tracing::error!("brain: {e:#}; falling back to the default local model");
        brain::Mind::new(&capri_core::Settings::default()).expect("default brain")
    });

    tauri::Builder::default()
        // A second launch just pokes the running one instead of starting another Capri.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app.emit("capri://poke", ());
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    let down = event.state() == ShortcutState::Pressed;
                    if down {
                        capri_core::journal::write("hotkey", serde_json::json!({ "key": format!("{:?}", shortcut.key) }));
                    }
                    if shortcut.key == Code::Escape {
                        if down {
                            listen::stop_job(app);
                        }
                        return;
                    }
                    let what = BOUND.lock().unwrap().iter().find(|(code, _)| shortcut.key == *code).map(|(_, w)| *w);
                    match what {
                        Some("talk") => listen::talk_key(app, down),
                        Some("wake_word") if down => {
                            listen::toggle_wake_word(app);
                        }
                        Some("mute") if down => {
                            listen::toggle_mute(app);
                        }
                        Some(what) if down => {
                            let _ = app.emit("capri://menu", what);
                        }
                        _ => {}
                    }
                })
                .build(),
        )
        .manage(App { settings: Mutex::new(settings), overlays: overlay::Overlays::default() })
        .manage(mind)
        .invoke_handler(tauri::generate_handler![testlog::log_everything, testlog::log_ui, overlay::set_hull, overlay::set_grab, overlay::screens_around, overlay::taskbar_top, overlay::fly_to, overlay::perches, overlay::possessable, overlay::shake_window, overlay::set_riding, overlay::follow_to, overlay::whereabouts, overlay::pointer_at, overlay::alive, overlay::hang_test, watch::mission_report, games::game_mode, brain::ask, brain::confirm, brain::forget, brain::set_mood, brain::bond_moment, brain::bond, brain::keys_status, brain::save_key, brain::delete_key, brain::online_brain, brain::plan_models, brain::key_page, speak::hush, listen::talk, remind_again, reminders_list, reminder_cancel, prefs::get_settings, prefs::idle_secs, prefs::now_playing, prefs::media_key, models::brain_status, hooks::connect_claude, models::recommended_brain, models::install_ollama, models::setup_pending, phone::save_phone_token, phone::delete_phone_token, phone::phone_status, phone::phone_hello, prefs::save_settings, prefs::local_models, beat_now, prefs::open_logs, prefs::notes_list, prefs::notes_remove, prefs::notes_clear, prefs::restart, prefs::version, open_settings, settings_ready, quit])
        .setup(move |app| {
            // First run: her speech models come down in the background (she restarts herself when they're here).
            // Her brain (Ollama there, her model pulled into it) is seen to once they're in: its question and its
            // download fought the speech models' progress line, and the question was gone before it could be read.
            if !models::fetch_missing(app.handle(), listen::models_dir(app.handle())) {
                let (handle, fast) = (app.handle().clone(), ears_settings.brain.fast.clone());
                tauri::async_runtime::spawn(async move { models::ensure_brain(&handle, &fast).await });
            }
            // Messages from your phone (Settings › Phone): waits quietly while that's off.
            phone::start(app.handle());
            // Her online brain gets ready now, not on your first request.
            let thinking = app.state::<brain::Mind>().brain();
            tauri::async_runtime::spawn(async move { thinking.warm().await });
            // An Esc she presses in an app goes to the app, not to her own Esc that stops the job.
            #[cfg(windows)]
            {
                let handle = app.handle().clone();
                let held = std::sync::atomic::AtomicBool::new(false);
                capri_hands::platform::around_own_keys(move |sending| {
                    use tauri_plugin_global_shortcut::GlobalShortcutExt;
                    let esc = tauri_plugin_global_shortcut::Shortcut::new(None, Code::Escape);
                    let shortcuts = handle.global_shortcut();
                    if sending {
                        let on = shortcuts.is_registered(esc);
                        held.store(on, std::sync::atomic::Ordering::Relaxed);
                        if on {
                            let _ = shortcuts.unregister(esc);
                        }
                    } else if held.swap(false, std::sync::atomic::Ordering::Relaxed) && listen::working(&handle) {
                        let _ = shortcuts.register(esc);
                    }
                });
            }
            app.manage(listen::start(app.handle(), &ears_settings));
            let models = listen::models_dir(app.handle());
            app.manage(speak::start(app.handle(), &ears_settings, models));
            overlay::open_all(app.handle())?;
            overlay::start_hit_test(app.handle().clone());
            overlay::watch_caret(app.handle().clone());
            brain::start_asides(app.handle());
            // Her blink-click hides your pointer for a twentieth of a second: if she ever died in it, it's back now.
            #[cfg(windows)]
            capri_hands::platform::restore_pointer();
            overlay::start_watchdog(app.handle().clone());
            games::start(app.handle().clone());
            reminders(app.handle().clone());
            // A day with her counts for her bond (once a day); a stage reached shows once she's up.
            let reached = capri_core::bond::add(None);
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(8));
                brain::bonded(&handle, reached);
            });
            watch::start(app.handle().clone());
            hooks::start(app.handle().clone());
            #[cfg(windows)]
            feel::start(app.handle().clone());
            care::start(app.handle().clone());
            // Her ear on the speakers: a steady beat makes her dance (in memory only, never kept).
            let handle = app.handle().clone();
            match capri_ears::beat::start(move |b| {
                *BEAT.lock().unwrap() = b;
                let _ = handle.emit("capri://beat", b);
            }) {
                Ok(l) => {
                    app.manage(BeatEar(std::sync::Mutex::new(Some(l))));
                }
                Err(e) => tracing::warn!("no ear for music: {e:#}"),
            }
            tray::build(app.handle())?;
            let settings = app.state::<App>().settings.lock().unwrap().clone();
            bind_keys(app.handle(), &settings);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Capricorn failed to start")
        .run(|app, event| {
            // Quitting (her menu, the tray): her model leaves the graphics card with her, not 15 minutes later.
            if let tauri::RunEvent::Exit = event
                && let Some(mind) = app.try_state::<brain::Mind>()
            {
                let brain = mind.brain();
                let _ = tauri::async_runtime::block_on(async { tokio::time::timeout(std::time::Duration::from_secs(3), brain.unload()).await });
                tracing::info!("quit: her models unloaded");
            }
        });
}

/// A reminder again in `minutes` (her "10 more min" button).
#[tauri::command]
fn remind_again(text: String, minutes: f64) -> Result<(), String> {
    let due = chrono::Local::now() + chrono::Duration::seconds((minutes * 60.0) as i64);
    capri_core::reminders::add(due, &text).map(|_| ()).map_err(|e| format!("{e:#}"))
}

/// Her reminders coming up, soonest first (her menu lists them).
#[tauri::command]
fn reminders_list() -> Vec<capri_core::reminders::Reminder> {
    capri_core::reminders::all()
}

/// Cancels one from her menu.
#[tauri::command]
fn reminder_cancel(id: u64) -> Result<(), String> {
    capri_core::reminders::remove(|r| r.id != id).map(|_| ()).map_err(|e| format!("{e:#}"))
}

/// Keeps her ear on the speakers open for as long as she runs.
struct BeatEar(#[allow(dead_code)] std::sync::Mutex<Option<capri_ears::beat::Listening>>);

/// Reminders and timers coming due: every few seconds, the ones due are said out loud and shown (`capri://remind`).
fn reminders(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("reminders".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let now = chrono::Local::now();
            for r in capri_core::reminders::take_due(now) {
                // Came due while she was off: she says so, with when.
                let missed = now - r.due > chrono::Duration::minutes(2);
                let what = r.text.trim_end_matches('.');
                // Something she was keeping an eye on happened ("Heads up: your download finished"): said as it is.
                let line = if let Some(news) = r.text.strip_prefix("Heads up: ") {
                    news.to_string()
                } else {
                    match (missed, what.is_empty()) {
                    (false, true) => "Time's up.".to_string(),
                    (false, false) => format!("You asked me to remind you: {what}."),
                    (true, true) => format!("A timer ran out while I was off, at {}.", r.due.format("%H:%M")),
                    (true, false) => format!("While I was off, at {}: {what}.", r.due.format("%H:%M")),
                    }
                };
                capri_core::journal::write("reminder", serde_json::json!({ "id": r.id, "text": r.text, "due": r.due.to_rfc3339() }));
                let _ = app.emit("capri://remind", serde_json::json!({ "line": line, "text": r.text }));
                speak::say(&app, &line, false);
            }
        })
        .expect("reminders thread");
}

/// Her log goes to the terminal and to `logs\capricorn-YYYY-MM-DD.log` (next to her journal); dev builds log more.
fn logging() {
    use tracing_subscriber::prelude::*;
    let default = if capri_core::journal::everything() {
        "info,capricorn_lib=debug,capri_brain=debug,capri_hands=debug,capri_core=debug,capri_ears=info"
    } else if cfg!(debug_assertions) {
        "info,capricorn_lib=debug,capri_brain=debug,capri_ears=info"
    } else {
        "info"
    };
    let filter = || tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| default.into());
    let dir = capri_core::journal::dir();
    let _ = std::fs::create_dir_all(&dir);
    let file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("capricorn-{}.log", chrono::Local::now().format("%Y-%m-%d"))));
    let console = tracing_subscriber::fmt::layer().with_filter(filter());
    match file {
        Ok(f) => {
            let to_file = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(std::sync::Mutex::new(f)).with_filter(filter());
            tracing_subscriber::registry().with(console).with(to_file).init();
        }
        Err(_) => tracing_subscriber::registry().with(console).init(),
    }
}

/// Async on purpose: a synchronous command runs on the main thread, and creating a window there deadlocks on Windows
/// (it froze her whole UI once: both overlays went white and covered the screens).
#[tauri::command]
async fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    open_settings_window(&app).map_err(|e| e.to_string())
}

pub(crate) fn open_settings_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("settings") {
        w.show()?;
        w.unminimize()?;
        return w.set_focus();
    }
    // Hidden, and her dark behind it, until the page has drawn itself: no white flash while it loads.
    let w = tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App("settings.html".into()))
        .title("Capricorn")
        .inner_size(960.0, 680.0)
        .min_inner_size(720.0, 520.0)
        .center()
        .visible(false)
        .background_color(tauri::window::Color(11, 6, 20, 255))
        .theme(Some(tauri::Theme::Dark))
        .build()?;
    // Her colours on the window's own frame too (shared/palette.css): ink title bar, white title, crimson border.
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        capri_hands::platform::tint_frame(h.0 as isize, 0x0b0614, 0xfbf8fc, 0xf60655);
    }
    // The page says when it's drawn (`settings_ready`); if it never does, the window shows anyway.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if !w.is_visible().unwrap_or(true) {
            let _ = w.show();
            let _ = w.set_focus();
        }
    });
    Ok(())
}

/// Her settings page has drawn itself: now it's shown.
#[tauri::command]
async fn settings_ready(window: tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    capri_core::journal::write("quit", serde_json::json!({ "from": "her menu" }));
    app.exit(0);
}
