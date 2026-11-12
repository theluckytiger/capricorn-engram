//! Her speech models, fetched on first run: the installer stays small, and what's missing (her ears' and her voice's
//! models, about 1 GB) comes from Hugging Face (or sherpa-onnx's releases on GitHub) into
//! `%LOCALAPPDATA%\capricorn-engram\models`, with the progress in her panel (`capri://setup`). When it's all there
//! she restarts herself, ears and voice on.
//!
//! Her brain's model is Ollama's business: if Ollama is there without it, she asks Ollama to pull it (same panel).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// One download: a file, or an archive unpacked into the models folder.
struct Need {
    what: &'static str,
    url: &'static str,
    /// A file that's there once it's done (relative to the models folder).
    gives: &'static str,
    /// The same model as separate files on Hugging Face (url, where it goes), tried first: GitHub's release downloads
    /// can crawl (36 KB/s against 1 MB/s from Hugging Face on the same connection). `gives` comes last.
    mirror: &'static [(&'static str, &'static str)],
}

const NEEDS: &[Need] = &[
    Need { what: "speech detection", url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx", gives: "silero_vad.onnx", mirror: &[] },
    Need {
        what: "my ears (Parakeet)",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
        gives: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/encoder.int8.onnx",
        mirror: &[
            ("https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/tokens.txt", "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/tokens.txt"),
            ("https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/decoder.int8.onnx", "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/decoder.int8.onnx"),
            ("https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/joiner.int8.onnx", "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/joiner.int8.onnx"),
            ("https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/encoder.int8.onnx", "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/encoder.int8.onnx"),
        ],
    },
    Need {
        what: "my voice (Kokoro)",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2",
        gives: "kokoro-multi-lang-v1_0/model.onnx",
        mirror: &[
            ("https://huggingface.co/csukuangfj/kokoro-multi-lang-v1_0/resolve/main/tokens.txt", "kokoro-multi-lang-v1_0/tokens.txt"),
            ("https://huggingface.co/csukuangfj/kokoro-multi-lang-v1_0/resolve/main/lexicon-us-en.txt", "kokoro-multi-lang-v1_0/lexicon-us-en.txt"),
            ("https://huggingface.co/csukuangfj/kokoro-multi-lang-v1_0/resolve/main/lexicon-gb-en.txt", "kokoro-multi-lang-v1_0/lexicon-gb-en.txt"),
            ("https://huggingface.co/csukuangfj/kokoro-multi-lang-v1_0/resolve/main/voices.bin", "kokoro-multi-lang-v1_0/voices.bin"),
            ("https://huggingface.co/csukuangfj/kokoro-multi-lang-v1_0/resolve/main/model.onnx", "kokoro-multi-lang-v1_0/model.onnx"),
        ],
    },
];

/// Where her models live: `CAPRI_MODELS`, next to the app (if a build ships them), the repo's `models/` (dev), else
/// her own folder (fetched there on first run).
pub fn dir(resource: Option<PathBuf>) -> PathBuf {
    if let Ok(dir) = std::env::var("CAPRI_MODELS") {
        return dir.into();
    }
    if let Some(res) = resource.map(|r| r.join("models"))
        && capri_ears::models_ok(&res)
    {
        return res;
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models");
    if cfg!(debug_assertions) && repo.is_dir() {
        return repo;
    }
    let local = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    local.join("capricorn-engram").join("models")
}

/// What's missing from `dir`.
fn missing(dir: &Path) -> Vec<&'static Need> {
    NEEDS.iter().filter(|n| !dir.join(n.gives).exists()).collect()
}

#[derive(Clone, Serialize)]
pub struct Progress {
    pub what: String,
    /// 0..1, or -1 while unpacking.
    pub done: f64,
    pub mb: u64,
    pub line: String,
}

/// Her question about installing Ollama, kept until it's answered: it can come before her page is listening (Ollama
/// missing is known in a few seconds, before her overlays have loaded), and the page asks for it once it is.
static ASKING: std::sync::Mutex<Option<Progress>> = std::sync::Mutex::new(None);

fn tell(app: &AppHandle, p: Progress) {
    if p.what == "ollama_missing" {
        *ASKING.lock().unwrap() = Some(p.clone());
    }
    let _ = app.emit("capri://setup", p);
}

/// The setup question still waiting for an answer, for a page that has just started listening.
#[tauri::command]
pub fn setup_pending() -> Option<Progress> {
    ASKING.lock().unwrap().clone()
}

/// On start: fetches what's missing in the background, then restarts her. True if anything was missing.
pub fn fetch_missing(app: &AppHandle, dir: PathBuf) -> bool {
    let todo = missing(&dir);
    if todo.is_empty() {
        return false;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        capri_core::journal::write("setup", serde_json::json!({ "fetching": todo.iter().map(|n| n.what).collect::<Vec<_>>(), "into": dir }));
        for need in todo {
            if let Err(e) = fetch(&app, &dir, need).await {
                tracing::error!("models: {}: {e:#}", need.what);
                capri_core::journal::write("error", serde_json::json!({ "where": "setup", "message": format!("{}: {e:#}", need.what) }));
                tell(&app, Progress { what: need.what.into(), done: 0.0, mb: 0, line: format!("Couldn't get {}: {e}. I'll try again next time you start me.", need.what) });
                return;
            }
        }
        tell(&app, Progress { what: "done".into(), done: 1.0, mb: 0, line: "Got my ears and voice. Restarting myself to use them.".into() });
        tokio::time::sleep(Duration::from_secs(3)).await;
        crate::prefs::relaunch(&app);
    });
    true
}

async fn fetch(app: &AppHandle, dir: &Path, need: &Need) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let name = need.url.rsplit('/').next().unwrap_or("download");
    let part = dir.join(format!("{name}.part"));
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(20)).user_agent("Capricorn").build()?;
    if !need.mirror.is_empty() {
        match fetch_files(app, &http, dir, need).await {
            Ok(()) if dir.join(need.gives).exists() => {
                capri_core::journal::write("setup", serde_json::json!({ "got": need.what, "from": "huggingface" }));
                return Ok(());
            }
            Ok(()) => tracing::warn!("{}: the files came but {} isn't there; trying GitHub", need.what, need.gives),
            Err(e) => tracing::warn!("{}: Hugging Face didn't work ({e:#}); trying GitHub", need.what),
        }
    }
    let resp = http.get(need.url).send().await?.error_for_status()?;
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&part).with_context(|| format!("writing {}", part.display()))?;
    let (mut got, mut said) = (0u64, Instant::now() - Duration::from_secs(1));
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        got += chunk.len() as u64;
        if said.elapsed() >= Duration::from_millis(400) {
            said = Instant::now();
            let done = if total > 0 { got as f64 / total as f64 } else { 0.0 };
            tell(app, Progress { what: need.what.into(), done, mb: got / 1_048_576, line: format!("Getting {}: {} of {} MB", need.what, got / 1_048_576, total / 1_048_576) });
        }
    }
    drop(file);
    if total > 0 && got != total {
        bail!("the download stopped at {got} of {total} bytes");
    }
    tell(app, Progress { what: need.what.into(), done: -1.0, mb: got / 1_048_576, line: format!("Unpacking {}…", need.what) });
    let (dir2, part2, name2) = (dir.to_path_buf(), part.clone(), name.to_string());
    tokio::task::spawn_blocking(move || -> Result<()> {
        if name2.ends_with(".tar.bz2") {
            let reader = bzip2::read::BzDecoder::new(std::io::BufReader::new(std::fs::File::open(&part2)?));
            tar::Archive::new(reader).unpack(&dir2)?;
            std::fs::remove_file(&part2)?;
        } else {
            std::fs::rename(&part2, dir2.join(&name2))?;
        }
        Ok(())
    })
    .await??;
    if !dir.join(need.gives).exists() {
        bail!("unpacked, but {} isn't there", need.gives);
    }
    capri_core::journal::write("setup", serde_json::json!({ "got": need.what, "mb": got / 1_048_576 }));
    Ok(())
}

/// A model as separate files from Hugging Face, each written under a `.part` name and renamed when whole, with one
/// progress line for all of them.
async fn fetch_files(app: &AppHandle, http: &reqwest::Client, dir: &Path, need: &Need) -> Result<()> {
    // A HEAD answer has no body, so `content_length()` says 0 ("2 of 0 MB"): the size is in its headers.
    let mut sizes = Vec::new();
    for (url, _) in need.mirror {
        let r = http.head(*url).send().await?.error_for_status()?;
        sizes.push(["content-length", "x-linked-size"].iter().find_map(|h| r.headers().get(*h)?.to_str().ok()?.parse::<u64>().ok()).unwrap_or(0));
    }
    let total: u64 = sizes.iter().sum();
    let (mut got, mut said) = (0u64, Instant::now() - Duration::from_secs(1));
    for ((url, rel), size) in need.mirror.iter().zip(sizes) {
        let dest = dir.join(rel);
        if dest.exists() {
            got += size;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let part = dest.with_file_name(format!("{}.part", dest.file_name().and_then(|n| n.to_str()).unwrap_or("download")));
        let mut file = std::fs::File::create(&part).with_context(|| format!("writing {}", part.display()))?;
        let mut body = http.get(*url).send().await?.error_for_status()?.bytes_stream();
        let mut this = 0u64;
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            file.write_all(&chunk)?;
            this += chunk.len() as u64;
            if said.elapsed() >= Duration::from_millis(400) {
                said = Instant::now();
                let now = got + this;
                tell(app, Progress { what: need.what.into(), done: if total > 0 { now as f64 / total as f64 } else { 0.0 }, mb: now / 1_048_576, line: format!("Getting {}: {} of {} MB", need.what, now / 1_048_576, total / 1_048_576) });
            }
        }
        drop(file);
        if size > 0 && this != size {
            bail!("{rel} stopped at {this} of {size} bytes");
        }
        std::fs::rename(&part, &dest)?;
        got += this;
    }
    Ok(())
}

/// Her brain's model in Ollama: pulled if Ollama is running without it (its own download, with progress).
fn ollama_base() -> String {
    std::env::var("OLLAMA_HOST").ok().filter(|h| !h.is_empty()).map(|h| if h.starts_with("http") { h } else { format!("http://{h}") }).unwrap_or_else(|| "http://127.0.0.1:11434".into())
}

/// Where Ollama is installed, if it is.
fn ollama_exe() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok().map(|d| PathBuf::from(d).join("Programs").join("Ollama").join("ollama.exe"));
    local.filter(|p| p.exists()).or_else(|| {
        let out = std::process::Command::new("where").arg("ollama").creation_flags_quiet().output().ok()?;
        String::from_utf8_lossy(&out.stdout).lines().next().map(|l| PathBuf::from(l.trim())).filter(|p| p.exists())
    })
}

trait Quiet {
    /// No console window flashing up (her app has none of its own).
    fn creation_flags_quiet(&mut self) -> &mut Self;
}

impl Quiet for std::process::Command {
    fn creation_flags_quiet(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(0x0800_0000);
        }
        self
    }
}

/// Her brain's state, for her settings window: Ollama installed, running, her model in it.
#[tauri::command]
pub async fn brain_status(state: tauri::State<'_, crate::App>) -> Result<serde_json::Value, String> {
    let fast = state.settings.lock().unwrap().brain.fast.clone();
    let tags: Option<serde_json::Value> = match reqwest::Client::new().get(format!("{}/api/tags", ollama_base())).timeout(Duration::from_secs(2)).send().await {
        Ok(r) => r.json().await.ok(),
        Err(_) => None,
    };
    let name = fast.strip_prefix("ollama/").unwrap_or("");
    let has = tags.as_ref().is_some_and(|t| t["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str()).any(|n| n == name || n == format!("{name}:latest")));
    Ok(serde_json::json!({ "local": fast.starts_with("ollama/"), "installed": ollama_exe().is_some() || tags.is_some(), "running": tags.is_some(), "model": name, "has_model": has }))
}

/// The local model that fits this PC, and why: by the graphics card's own memory (the model and what it's reading
/// must fit there to be quick: 8B is 5.9 GB, 4B 3.2 GB, 2B 1.8 GB), else by the PC's memory (on the processor, slower).
pub fn recommended() -> (String, String) {
    let gb = |b: u64| b as f64 / 1_073_741_824.0;
    #[cfg(windows)]
    let (card, ram) = (capri_hands::platform::system::graphics_memory(), capri_hands::platform::system::memory().map_or(0, |m| m.1));
    #[cfg(not(windows))]
    let (card, ram): (Option<(String, u64)>, u64) = (None, 0);
    let (size, why) = match &card {
        Some((name, v)) if gb(*v) >= 7.5 => ("8b", format!("{name}, {:.0} GB of its own memory", gb(*v))),
        Some((name, v)) if gb(*v) >= 4.5 => ("4b", format!("{name}, {:.0} GB of its own memory", gb(*v))),
        Some((name, v)) if gb(*v) >= 2.5 => ("2b", format!("{name}, {:.0} GB of its own memory", gb(*v))),
        _ if gb(ram) >= 24.0 => ("4b", format!("no graphics card with memory of its own; {:.0} GB of memory, so on the processor", gb(ram))),
        _ => ("2b", format!("no graphics card with memory of its own; {:.0} GB of memory, so on the processor", gb(ram))),
    };
    (format!("ollama/qwen3-vl:{size}-instruct"), why)
}

/// For her settings: the model she'd pick for this PC, and why.
#[tauri::command(async)]
pub fn recommended_brain() -> serde_json::Value {
    let (model, why) = recommended();
    serde_json::json!({ "model": model, "why": why })
}

/// Ollama's own Windows installer (it redirects to the newest release).
const OLLAMA_SETUP: &str = "https://ollama.com/download/OllamaSetup.exe";

/// Gets Ollama's installer (near 2 GB) into her temp folder, with her usual progress line. winget did this before and
/// said nothing while it downloaded; on a slow line its download stopped at 6 MB and sat there. Here a connection that
/// stops sending is dropped after a minute and picked up where it stopped, and a download cut short (she was quit, the
/// PC slept) carries on from its `.part` file the next time.
async fn get_ollama_setup(app: &AppHandle) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("capricorn-ollama");
    std::fs::create_dir_all(&dir)?;
    let setup = dir.join("OllamaSetup.exe");
    if setup.exists() {
        return Ok(setup);
    }
    let part = dir.join("OllamaSetup.exe.part");
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(20)).user_agent("Capricorn").build()?;
    let mut said = Instant::now() - Duration::from_secs(1);
    let mut fails = 0;
    loop {
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let attempt: Result<bool> = async {
            let mut req = http.get(OLLAMA_SETUP);
            if have > 0 {
                req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
            }
            let resp = req.send().await?;
            // Asked for the rest of a file that's already whole.
            if have > 0 && resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                return Ok(true);
            }
            let resp = resp.error_for_status()?;
            let resumed = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
            let (mut got, mut file) = if resumed { (have, std::fs::OpenOptions::new().append(true).open(&part)?) } else { (0, std::fs::File::create(&part)?) };
            let length = resp.content_length();
            let total = got + length.unwrap_or(0);
            let mut body = resp.bytes_stream();
            while let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), body.next()).await.context("the download stopped sending")? {
                let chunk = chunk?;
                file.write_all(&chunk)?;
                got += chunk.len() as u64;
                if said.elapsed() >= Duration::from_millis(500) {
                    said = Instant::now();
                    let line = if total > got { format!("Getting Ollama: {} of {} MB", got / 1_048_576, total / 1_048_576) } else { format!("Getting Ollama: {} MB", got / 1_048_576) };
                    tell(app, Progress { what: "ollama".into(), done: if total > 0 { got as f64 / total as f64 } else { -1.0 }, mb: got / 1_048_576, line });
                }
            }
            // Whole when it's as long as it said, or ended cleanly without saying.
            Ok(length.is_none() || got >= total)
        }
        .await;
        // Thirty tries in a row that got nothing more: it isn't coming.
        let moved = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0) > have;
        fails = if moved { 0 } else { fails + 1 };
        match attempt {
            Ok(true) => break,
            // Cut off without an error: the rest, from where it stopped.
            Ok(false) if fails <= 30 => {}
            Ok(false) => bail!("Ollama's installer keeps stopping"),
            Err(e) => {
                tracing::warn!("Ollama's installer: {e:#} (tries in a row: {fails})");
                if fails > 30 {
                    return Err(e);
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    std::fs::rename(&part, &setup)?;
    Ok(setup)
}

/// Installs Ollama (its own installer, for this user only: no admin), starts it, then gets her model. Progress as
/// `capri://setup`.
#[tauri::command]
pub async fn install_ollama(app: AppHandle) -> Result<(), String> {
    *ASKING.lock().unwrap() = None;
    capri_core::journal::write("setup", serde_json::json!({ "installing": "ollama" }));
    if ollama_exe().is_none() {
        tell(&app, Progress { what: "ollama".into(), done: -1.0, mb: 0, line: "Getting Ollama, my local brain. It's a big one, near 2 GB.".into() });
        let installed = match get_ollama_setup(&app).await {
            Ok(setup) => {
                tell(&app, Progress { what: "ollama".into(), done: -1.0, mb: 0, line: "Installing Ollama.".into() });
                let run = setup.clone();
                let ok = tokio::task::spawn_blocking(move || std::process::Command::new(&run).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"]).creation_flags_quiet().status().is_ok_and(|s| s.success()))
                    .await
                    .unwrap_or(false);
                // Installed, its installer isn't needed; not installed, it may be what's wrong: either way it goes.
                let _ = std::fs::remove_file(&setup);
                ok && ollama_exe().is_some()
            }
            Err(e) => {
                tracing::error!("Ollama's installer: {e:#}");
                false
            }
        };
        if !installed {
            let _ = std::process::Command::new("explorer.exe").arg("https://ollama.com/download").spawn();
            tell(&app, Progress { what: "ollama".into(), done: 0.0, mb: 0, line: "Couldn't install it myself. I opened ollama.com: get it there, then start me again.".into() });
            capri_core::journal::write("setup", serde_json::json!({ "ollama": "install failed" }));
            return Err("couldn't install Ollama".into());
        }
        capri_core::journal::write("setup", serde_json::json!({ "ollama": "installed" }));
    }
    // Running? If not, start it (in the background, no window).
    if reqwest::Client::new().get(format!("{}/api/tags", ollama_base())).timeout(Duration::from_secs(2)).send().await.is_err()
        && let Some(exe) = ollama_exe()
    {
        let _ = std::process::Command::new(exe).arg("serve").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).creation_flags_quiet().spawn();
    }
    let fast = app.state::<crate::App>().settings.lock().unwrap().brain.fast.clone();
    ensure_brain(&app, &fast).await;
    Ok(())
}

pub async fn ensure_brain(app: &AppHandle, model_id: &str) {
    let Some(name) = model_id.strip_prefix("ollama/") else { return };
    let http = reqwest::Client::new();
    let base = ollama_base();
    // Ollama may still be starting, or be on this PC with nobody having started it (she told you it wasn't here and
    // offered to install it): started if it's installed, and given a while to answer.
    let installed = ollama_exe();
    let mut tags = None;
    for i in 0..30 {
        if let Ok(r) = http.get(format!("{base}/api/tags")).send().await {
            tags = r.json::<serde_json::Value>().await.ok();
            break;
        }
        if i == 2
            && let Some(exe) = &installed
        {
            capri_core::journal::write("setup", serde_json::json!({ "ollama": "not running: starting it" }));
            let _ = std::process::Command::new(exe).arg("serve").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).creation_flags_quiet().spawn();
        }
        // Not installed: nothing will start it, and each refused try takes Windows a couple of seconds.
        if i >= 1 && installed.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if tags.is_none() && installed.is_some() {
        capri_core::journal::write("error", serde_json::json!({ "where": "setup", "message": "Ollama is installed but didn't answer in 30 s" }));
        return;
    }
    let Some(tags) = tags else {
        tell(app, Progress { what: "ollama_missing".into(), done: 0.0, mb: 0, line: "My local brain runs on Ollama, and it isn't here. Want me to install it? (Or let me think online, or give me a cloud key: my settings, Brain.)".into() });
        capri_core::journal::write("setup", serde_json::json!({ "ollama": "missing" }));
        return;
    };
    let have = tags["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str()).any(|n| n == name || n == format!("{name}:latest"));
    if have {
        warm(&http, &base, name).await;
        return;
    }
    capri_core::journal::write("setup", serde_json::json!({ "pulling": name }));
    let Ok(resp) = http.post(format!("{base}/api/pull")).json(&serde_json::json!({ "model": name, "stream": true })).send().await else { return };
    let mut body = resp.bytes_stream();
    let mut said = Instant::now() - Duration::from_secs(1);
    let mut buf = Vec::new();
    // The model comes in pieces, one after the other (the weights, then the part that sees): each counts on top of
    // the ones before, so the line never drops back to zero halfway.
    let mut pieces: std::collections::HashMap<String, (u64, u64)> = std::collections::HashMap::new();
    while let Some(Ok(chunk)) = body.next().await {
        buf.extend_from_slice(&chunk);
        while let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line) else { continue };
            if let (Some(piece), Some(total)) = (v["digest"].as_str(), v["total"].as_u64()) {
                pieces.insert(piece.to_string(), (v["completed"].as_u64().unwrap_or(0), total));
            }
            let (got, total) = pieces.values().fold((0, 0), |(g, t), (pg, pt)| (g + pg, t + pt));
            if total > 0 && said.elapsed() >= Duration::from_millis(500) {
                said = Instant::now();
                tell(app, Progress { what: "brain".into(), done: got as f64 / total as f64, mb: got / 1_048_576, line: format!("Getting my brain ({name}): {} of {} MB", got / 1_048_576, total / 1_048_576) });
            }
            if v["status"] == "success" {
                capri_core::journal::write("setup", serde_json::json!({ "pulled": name }));
                // Loaded before she says it's here: the first question after setup took 29 s of loading.
                warm(&http, &base, name).await;
                tell(app, Progress { what: "done".into(), done: 1.0, mb: 0, line: "My brain's here. Ask me anything.".into() });
            }
        }
    }
}

/// Her model loaded onto the graphics card now rather than on your first request (12 to 30 s of it), unless a game
/// has the graphics card.
async fn warm(http: &reqwest::Client, base: &str, name: &str) {
    #[cfg(windows)]
    if capri_hands::platform::front_fullscreen() {
        return;
    }
    let started = Instant::now();
    // With the context size her calls use: loaded at Ollama's default, her first request loaded it all over again.
    let options = serde_json::json!({ "num_ctx": capri_brain::providers::OLLAMA_NUM_CTX });
    let _ = http.post(format!("{base}/api/generate")).json(&serde_json::json!({ "model": name, "keep_alive": "2h", "options": options })).send().await;
    capri_core::journal::write("setup", serde_json::json!({ "warmed": name, "ms": started.elapsed().as_millis() as u64 }));
}
