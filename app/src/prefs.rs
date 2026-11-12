//! Her settings window's side: read her settings, save them (applying what can change on the spot), list the models
//! Ollama has, open her logs folder, restart her.

use std::sync::atomic::Ordering;

use capri_core::Settings;
use tauri::{AppHandle, Emitter, Manager, State};

/// Her settings as they are now (what's on disk, which is what she started with plus any change since).
#[tauri::command]
pub fn get_settings(state: State<'_, crate::App>) -> Settings {
    Settings::load().unwrap_or_else(|_| state.settings.lock().unwrap().clone())
}

/// Saves them. Voice on/off and listening for her name change at once; the rest (her models, conversation mode, her
/// voice itself) on her next start: the answer says whether a restart is needed.
#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<'_, crate::App>, settings: Settings) -> Result<bool, String> {
    let before = state.settings.lock().unwrap().clone();
    // A model chosen by hand: she stops picking one for this PC.
    let mut settings = settings;
    if settings.brain.fast != before.brain.fast {
        settings.brain.auto = false;
    }
    settings.save().map_err(|e| format!("{e:#}"))?;
    // Now: her voice on or off, her ears listening for her name or not.
    app.state::<crate::speak::Voice>().enabled.store(settings.voice.enabled, Ordering::Relaxed);
    let l = app.state::<crate::listen::Listening>();
    l.set_follow_up(settings.ears.follow_up_secs);
    if l.wake_word_on() != settings.ears.wake_word {
        crate::listen::toggle_wake_word(&app);
    }
    let restart = before.brain.fast != settings.brain.fast
        || before.brain.smart != settings.brain.smart
        || before.brain.eyes != settings.brain.eyes
        || before.brain.cloud != settings.brain.cloud
        || before.home != settings.home
        || before.voice.engine != settings.voice.engine
        || before.voice.speaker != settings.voice.speaker
        || before.voice.speed != settings.voice.speed
        || before.privacy.screenshots_to_cloud != settings.privacy.screenshots_to_cloud;
    // Her keys, rebound at once.
    if before.keys != settings.keys {
        let taken = crate::bind_keys(&app, &settings);
        if !taken.is_empty() {
            capri_core::journal::write("keys", serde_json::json!({ "taken": taken }));
        }
    }
    let changed = crate::testlog::changes(&serde_json::to_value(&before).unwrap_or_default(), &serde_json::to_value(&settings).unwrap_or_default());
    capri_core::journal::write("settings", serde_json::json!({ "saved": true, "restart": restart, "changed": changed }));
    *state.settings.lock().unwrap() = settings;
    let _ = app.emit("capri://settings", ());
    Ok(restart)
}

/// The models on this PC: Ollama's as `ollama/name`, and LM Studio's, llama.cpp's or vLLM's as `lmstudio/id`… when one is
/// running on its usual port (empty when none is).
#[tauri::command]
pub async fn local_models() -> Vec<String> {
    let base = std::env::var("OLLAMA_HOST").ok().filter(|h| !h.is_empty()).map(|h| if h.starts_with("http") { h } else { format!("http://{h}") }).unwrap_or_else(|| "http://127.0.0.1:11434".into());
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_millis(1500)).build().unwrap_or_default();
    let ollama = async {
        let Ok(r) = http.get(format!("{base}/api/tags")).send().await else { return vec![] };
        let v: serde_json::Value = r.json().await.unwrap_or_default();
        v["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str()).map(|n| format!("ollama/{n}")).collect::<Vec<_>>()
    };
    // The other servers on this PC that speak OpenAI's API, if one is running: their loaded models.
    let server = |prefix: &'static str, port: u16| {
        let http = http.clone();
        async move {
            let Ok(r) = http.get(format!("http://127.0.0.1:{port}/v1/models")).send().await else { return vec![] };
            let v: serde_json::Value = r.json().await.unwrap_or_default();
            v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).map(|id| format!("{prefix}/{id}")).collect::<Vec<_>>()
        }
    };
    let (a, b, c, d) = tokio::join!(ollama, server("lmstudio", 1234), server("llamacpp", 8080), server("vllm", 8000));
    [a, b, c, d].concat()
}

/// Seconds since you last touched the keyboard or mouse (her check-ins only come while you're there).
#[tauri::command]
pub fn idle_secs() -> u64 {
    #[cfg(windows)]
    return capri_hands::platform::system::idle().as_secs();
    #[cfg(not(windows))]
    0
}

/// Opens her logs folder in Explorer.
#[tauri::command]
pub fn open_logs() -> Result<(), String> {
    let dir = capri_core::journal::dir();
    std::process::Command::new("explorer.exe").arg(&dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn restart(app: AppHandle) {
    capri_core::journal::write("restart", serde_json::json!({ "from": "Settings" }));
    // A dev build loads her pages from the dev server, which goes when she does: relaunched alone she came back to
    // "localhost refused to connect". Started from a launcher script in the repo folder (`start-capri.bat`, if there is
    // one), that runs again a few seconds after she's gone, and brings both back.
    #[cfg(all(debug_assertions, windows))]
    {
        let launcher = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../start-capri.bat");
        if let Ok(launcher) = launcher.canonicalize() {
            use std::os::windows::process::CommandExt;
            // The line goes to cmd exactly as written (passed as an argument, its quotes were escaped and Windows
            // looked for a program called \Capri\). `ping` waits the few seconds (`timeout` needs a console, and this
            // one has none: no window flashes up for it).
            let line = format!("/c ping -n 5 127.0.0.1 >nul & start \"Capri\" \"{}\"", launcher.display().to_string().trim_start_matches(r"\\?\"));
            if std::process::Command::new("cmd.exe").raw_arg(&line).creation_flags(0x0800_0000).spawn().is_ok() {
                app.exit(0);
                return;
            }
        }
    }
    relaunch(&app);
}

/// Starts her again, as a new process a few seconds from now. Tauri's own restart, called from her first-start
/// download, hung her main thread on the way out: the overlays hid, the process stayed and nothing came back. And a
/// new copy started while this one is still going is turned away as a second instance. So a hidden `cmd` waits four
/// seconds, ends this process if it's somehow still there, and starts her; meanwhile she quits the normal way.
pub fn relaunch(app: &AppHandle) {
    capri_core::journal::write("restart", serde_json::json!({ "how": "relaunch" }));
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        use std::os::windows::process::CommandExt;
        let exe = exe.display().to_string();
        let name = std::path::Path::new(&exe).file_name().and_then(|n| n.to_str()).unwrap_or("capricorn.exe").to_string();
        let line = format!(
            "/c ping -n 5 127.0.0.1 >nul & taskkill /F /FI \"PID eq {}\" /FI \"IMAGENAME eq {name}\" >nul 2>&1 & start \"\" \"{}\"",
            std::process::id(),
            exe.trim_start_matches(r"\\?\")
        );
        if let Err(e) = std::process::Command::new("cmd.exe").raw_arg(&line).creation_flags(0x0800_0000).spawn() {
            tracing::error!("couldn't set up her restart: {e}");
        }
    }
    app.exit(0);
}

#[tauri::command]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Her notes on how things worked in your apps (Settings › Memory).
#[tauri::command]
pub fn notes_list() -> Vec<capri_core::notes::Note> {
    capri_core::notes::all()
}

#[tauri::command]
pub fn notes_remove(index: usize) -> Result<(), String> {
    capri_core::notes::remove(index).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn notes_clear() -> Result<(), String> {
    capri_core::notes::clear().map_err(|e| format!("{e:#}"))
}

/// What's playing (the media session Windows puts first, else any playing one), for her mini player.
#[tauri::command(async)]
pub fn now_playing() -> Option<serde_json::Value> {
    #[cfg(windows)]
    {
        let all = capri_hands::platform::now_playing().ok()?;
        let n = all.into_iter().find(|n| !n.title.trim().is_empty())?;
        Some(serde_json::json!({ "title": n.title, "artist": n.artist, "app": n.app, "playing": n.playing }))
    }
    #[cfg(not(windows))]
    None
}

/// Her mini player's keys: previous, toggle, next (to the app playing it).
#[tauri::command(async)]
pub fn media_key(action: String) -> Result<(), String> {
    if !matches!(action.as_str(), "previous" | "toggle" | "next" | "play" | "pause") {
        return Err(format!("unknown key {action}"));
    }
    #[cfg(windows)]
    return capri_hands::platform::media(&action).map(|_| ()).map_err(|e| format!("{e:#}"));
    #[cfg(not(windows))]
    Err("not on this OS".into())
}
