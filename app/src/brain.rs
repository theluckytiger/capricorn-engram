//! Her brain inside the app: requests come from her panel (typed) or her ears (spoken), steps and answers go back
//! as events, and risky steps wait for your click on her panel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use capri_brain::{Answer, Brain, Events, Gate, StepEvent, providers};
use capri_hands::Risk;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

pub struct Mind {
    brain: Arc<Brain>,
    /// What her tools noticed on their own (watching along), to be said her way; taken by `start_asides`.
    asides: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<String>>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next: AtomicU64,
}

impl Mind {
    pub fn new(settings: &capri_core::Settings) -> anyhow::Result<Self> {
        // Keys: an environment variable first, else one saved in Windows' Credential Manager (Settings › Brain).
        let env = providers::env_keys();
        #[cfg(windows)]
        let keys: providers::KeyLookup = std::sync::Arc::new(move |p: &str| env(p).or_else(|| capri_hands::platform::vault::key_get(p)));
        #[cfg(not(windows))]
        let keys = env;
        // A model that can't be used (its key removed, a service she no longer knows): she thinks locally instead.
        let mut fast = providers::model(&settings.brain.fast, &keys).or_else(|e| {
            tracing::warn!("{}: {e:#}; thinking locally instead", settings.brain.fast);
            providers::model(&capri_core::Settings::default().brain.fast, &keys)
        })?;
        // A model in the cloud thinks for her only if the cloud is allowed (Settings › Brain): else nothing leaves this
        // PC, and she thinks with the local one.
        if !fast.local() && !settings.brain.cloud {
            tracing::warn!("{} is online and the cloud is off: thinking locally instead", fast.id());
            fast = providers::model(&capri_core::Settings::default().brain.fast, &keys)?;
        }
        // The one you picked, and only that one: she doesn't move to another smart brain by herself. If it can't answer, her
        // basic brain on this PC does the job.
        let smart = settings.brain.smart.as_ref().and_then(|id| providers::model(id, &keys).inspect_err(|e| tracing::warn!("smart model {id}: {e:#}")).ok());
        // Her eyes: the `eyes` model (the same one as `fast` by default), and only if she may show it the screen.
        let eyes = match &settings.brain.eyes {
            Some(id) if *id == settings.brain.fast => Some(fast.clone()),
            Some(id) => providers::model(id, &keys).inspect_err(|e| tracing::warn!("eyes model {id}: {e:#}")).ok(),
            None => None,
        };
        let eyes = eyes.filter(|m| {
            let allowed = m.local() || settings.privacy.screenshots_to_cloud;
            if !allowed {
                tracing::warn!("eyes model {} is in the cloud and screenshots may not go there: her eyes are off", m.id());
            }
            allowed
        });
        let (aside_tx, aside_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let aside = capri_hands::Aside(Arc::new(move |seen: String| {
            let _ = aside_tx.send(seen);
        }));
        let ctx = capri_hands::Ctx { home: settings.home.clone(), own_pid: std::process::id(), dry_run: std::env::var("CAPRI_DRY").is_ok(), screens_to_cloud: settings.privacy.screenshots_to_cloud, aside: Some(aside), ..Default::default() };
        let mut brain = Brain::new(capri_hands::all(), ctx, fast, smart, settings.brain.cloud);
        if let Some(e) = eyes {
            brain = brain.with_eyes(e);
        }
        // Eyes in the cloud, asked for each time (never without your yes): the stronger brain when it sees pictures,
        // or eyes set to a cloud model that screenshots may not go to on their own.
        // (Not the 8B on this PC, which is also a "-vl": it took this place, and the online brain's eyes were never offered.)
        let on_this_pc = |id: &str| ["ollama/", "lmstudio/", "llamacpp/", "vllm/"].iter().any(|p| id.starts_with(p));
        // (Not Cursor's agent either: about a minute a picture.)
        let sees = |id: &str| !on_this_pc(id) && (["gemini/", "anthropic/", "openai/", "claudecode/"].iter().any(|p| id.starts_with(p)) || id.contains("llama-4") || id.contains("-vl"));
        let cloud_eyes = [settings.brain.eyes.as_deref(), settings.brain.smart.as_deref(), Some(settings.brain.fast.as_str())].into_iter().flatten().find(|id| sees(id) && settings.brain.cloud && !settings.privacy.screenshots_to_cloud);
        if let Some(id) = cloud_eyes
            && let Ok(m) = providers::model(id, &keys)
        {
            let name = match id.split('/').next().unwrap_or_default() {
                "gemini" => "Gemini",
                "anthropic" => "Claude",
                "openai" => "OpenAI",
                "groq" => "Groq",
                "claudecode" => "Claude",
                "nvidia" => "NVIDIA",
                "cursor" => "Cursor",
                other => other,
            };
            brain = brain.with_cloud_eyes(m, name);
        }
        brain.daily_cloud = settings.brain.daily_cloud_requests;
        Ok(Self { brain: Arc::new(brain), asides: Mutex::new(Some(aside_rx)), pending: Mutex::new(HashMap::new()), next: AtomicU64::new(1) })
    }
}

impl Mind {
    pub fn brain(&self) -> Arc<Brain> {
        self.brain.clone()
    }
}

/// What she notices on her own while watching along: her reaction, said and shown (once, at her start).
pub fn start_asides(app: &AppHandle) {
    let Some(mut rx) = app.state::<Mind>().asides.lock().unwrap().take() else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(seen) = rx.recv().await {
            let brain = app.state::<Mind>().brain();
            let Some(line) = brain.aside(&seen).await else { continue };
            capri_core::journal::write("aside", serde_json::json!({ "seen": seen, "line": line }));
            crate::speak::say(&app, &line, false);
            let _ = app.emit("capri://feel", serde_json::json!({ "kind": "aside", "line": line, "speak": true }));
        }
    });
}

#[derive(Clone, Serialize)]
struct Confirm {
    id: u64,
    what: String,
    risk: Risk,
}

struct UiGate {
    app: AppHandle,
}

#[async_trait]
impl Gate for UiGate {
    async fn allow(&self, what: &str, risk: Risk) -> bool {
        // Blackwall mode: risky goes ahead without asking (never-silent still asks).
        if risk == Risk::Risky && self.app.state::<crate::App>().settings.lock().unwrap().blackwall {
            capri_core::journal::write("confirm", serde_json::json!({ "what": what, "risk": risk, "yes": true, "blackwall": true }));
            return true;
        }
        let mind = self.app.state::<Mind>();
        let id = mind.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        mind.pending.lock().unwrap().insert(id, tx);
        let _ = self.app.emit("capri://confirm", Confirm { id, what: what.to_string(), risk });
        // Said out loud too, and her ears stay open for "yes" or "no" until it's answered: a "Yes." said to it was
        // taken as not for her (her name wasn't in it).
        let question = what.split(" (").next().unwrap_or(what).trim().to_string();
        // Said as a question: "Press “Install” in Steam." sounded like she was telling you to, and the "No." it got
        // meant "you do it".
        let question = match question.strip_suffix('.') {
            Some(q) if !question.ends_with('?') => {
                let mut c = q.chars();
                c.next().map(|f| format!("Should I {}{}?", f.to_lowercase(), c.as_str())).unwrap_or(question.clone())
            }
            _ => question,
        };
        crate::speak::say(&self.app, &question, false);
        crate::listen::listen_for_answer(&self.app);
        crate::listen::waiting_for_answer(&self.app, true);
        // No answer in a minute counts as no.
        let yes = matches!(tokio::time::timeout(Duration::from_secs(60), rx).await, Ok(Ok(true)));
        crate::listen::waiting_for_answer(&self.app, false);
        mind.pending.lock().unwrap().remove(&id);
        capri_core::journal::write("confirm", serde_json::json!({ "what": what, "risk": risk, "yes": yes }));
        yes
    }
}

struct UiEvents {
    app: AppHandle,
    /// The request these steps belong to, in her journal.
    id: u64,
}

impl Events for UiEvents {
    fn step(&self, ev: &StepEvent) {
        *self.app.state::<crate::listen::Listening>().moved.lock().unwrap() = std::time::Instant::now();
        // The event's own "kind" (thinking/started/finished) becomes "step", so the journal line stays a "step".
        let mut v = serde_json::to_value(ev).unwrap_or_default();
        if let Some(obj) = v.as_object_mut() {
            let step = obj.remove("kind").unwrap_or_default();
            obj.insert("step".into(), step);
            obj.insert("id".into(), self.id.into());
        }
        capri_core::journal::write("step", v);
        let _ = self.app.emit("capri://step", ev);
    }

    fn note(&self, what: &str, data: serde_json::Value) {
        tracing::debug!("#{} {what} {data}", self.id);
        capri_core::journal::write("note", serde_json::json!({ "id": self.id, "what": what, "data": data }));
    }

    fn request(&self) -> u64 {
        self.id
    }

    fn heard(&self) -> Vec<String> {
        self.app.state::<crate::listen::Listening>().told.lock().unwrap().drain(..).collect()
    }
}

#[derive(Clone, Serialize)]
struct Spoken<'a> {
    #[serde(flatten)]
    answer: &'a Answer,
    /// The request was said out loud (her panel shows what she heard; her ears stay open for a follow-up).
    spoken: bool,
}

/// One request, typed or spoken, all the way to her answer (sent to her overlay as `capri://answer`).
pub async fn answer(app: &AppHandle, text: &str, spoken: bool) -> Answer {
    let answer = run(app, text, spoken, &UiGate { app: app.clone() }, None).await;
    // Her name came while she worked on this: it waits until she stops listening (a new request replaces it).
    if !app.state::<crate::listen::Listening>().hold(&answer, spoken) {
        deliver(app, &answer, spoken);
    }
    answer
}

/// A message from your phone (`crate::phone`): done like a typed request and shown on her screen, but not said out
/// loud (you're not at the PC); risky steps are asked in the chat (`gate`). The caller sends the answer back.
pub async fn answer_phone(app: &AppHandle, text: &str, gate: &dyn Gate) -> Answer {
    let answer = run(app, text, false, gate, Some("discord")).await;
    let _ = app.emit("capri://answer", Spoken { answer: &answer, spoken: false });
    answer
}

/// A request start to end: her brain on it, her journal, her bond, her settings if it changed them. Not shown or said.
async fn run(app: &AppHandle, text: &str, spoken: bool, gate: &dyn Gate, from: Option<&str>) -> Answer {
    let brain = app.state::<Mind>().brain.clone();
    let id = capri_core::journal::next_id();
    tracing::info!("asked{}{}: {text}", if spoken { " (spoken)" } else { "" }, from.map(|f| format!(" (from {f})")).unwrap_or_default());
    capri_core::journal::write("request", serde_json::json!({ "id": id, "text": text, "spoken": spoken, "from": from }));
    let events = UiEvents { app: app.clone(), id };
    let mut answer = brain.ask(text, gate, &events).await;
    // "Go to my other screen": she moves, or there's only one monitor.
    if answer.via == "screen" && !crate::overlay::go_other_screen(app) {
        answer.line = "I've only got one screen to stand on.".into();
    }
    tracing::info!("[{} {:?} {} ms] {}", answer.via, answer.model, answer.ms, answer.line);
    capri_core::journal::write(
        "answer",
        serde_json::json!({
            "id": id,
            "line": answer.line,
            "via": answer.via,
            "model": answer.model,
            "ms": answer.ms,
            // `data`: what the tool saw on the way (the controls it chose from, the switch states before and after…).
            "steps": answer.steps.iter().map(|s| serde_json::json!({ "tool": s.tool, "args": s.args, "status": s.outcome.status, "summary": s.outcome.summary, "data": s.outcome.data, "ms": s.ms })).collect::<Vec<_>>(),
        }),
    );
    // Her bond: a chat, or a job that got done.
    let moment = if answer.via == "chat" {
        Some(capri_core::bond::Moment::Chat)
    } else if answer.steps.iter().any(|s| s.outcome.status == capri_hands::Status::Ok) {
        Some(capri_core::bond::Moment::Job)
    } else {
        None
    };
    if let Some(m) = moment {
        bonded(app, capri_core::bond::add(Some(m)));
    }
    // "Call me…", "remember that…" changed her settings: her settings window shows it.
    if answer.via == "yours" {
        if let Ok(s) = capri_core::Settings::load() {
            *app.state::<crate::App>().settings.lock().unwrap() = s;
        }
        let _ = app.emit("capri://settings", ());
    }
    answer
}

/// Her answer, shown and said; her ears open again for your reply.
pub fn deliver(app: &AppHandle, answer: &Answer, spoken: bool) {
    let _ = app.emit("capri://answer", Spoken { answer, spoken });
    let said = crate::speak::say(app, &answer.line, spoken);
    // Not said out loud (her voice is off): listening for your answer starts now instead of when she's done.
    if spoken && !said {
        let l = app.state::<crate::listen::Listening>();
        if answer.line.trim_end().ends_with('?') {
            crate::listen::listen_for_answer(app);
        } else if let Some(e) = &l.ears {
            l.follow_up(app, e);
        }
    }
}

/// She just reached a stage of her bond: her overlay makes it a moment.
pub fn bonded(app: &AppHandle, reached: Option<u8>) {
    if let Some(stage) = reached {
        let name = capri_core::bond::STAGES[(stage as usize).min(4)].0;
        capri_core::journal::write("bond", serde_json::json!({ "stage": stage, "name": name }));
        let _ = app.emit("capri://bond", serde_json::json!({ "stage": stage, "name": name }));
    }
}

/// A moment for her bond from her overlay: a reminder done, a head pat.
#[tauri::command]
pub fn bond_moment(app: AppHandle, what: String) {
    let m = match what.as_str() {
        "reminder" => capri_core::bond::Moment::Reminder,
        "pat" => capri_core::bond::Moment::Pat,
        _ => return,
    };
    bonded(&app, capri_core::bond::add(Some(m)));
}

/// The cloud providers, and where each one's key is (never the key): saved, in the environment, or nowhere.
#[tauri::command]
pub fn keys_status() -> Vec<serde_json::Value> {
    providers::cloud_providers()
        .into_iter()
        .map(|(id, name)| {
            let env = providers::key_env(id).and_then(|v| std::env::var(v).ok()).is_some_and(|k| !k.trim().is_empty());
            #[cfg(windows)]
            let saved = capri_hands::platform::vault::key_get(id).is_some();
            #[cfg(not(windows))]
            let saved = false;
            serde_json::json!({ "id": id, "name": name, "saved": saved, "env": env, "env_var": providers::key_env(id) })
        })
        .collect()
}

/// A key pasted in her settings: checked with a call that needs it, then saved encrypted (Credential Manager).
#[tauri::command]
pub async fn save_key(provider: String, key: String) -> Result<String, String> {
    let key = key.trim().to_string();
    providers::check_key(&provider, &key).await.map_err(|e| format!("That key didn't work: {e:#}"))?;
    #[cfg(windows)]
    capri_hands::platform::vault::key_set(&provider, &key).map_err(|e| format!("{e:#}"))?;
    capri_core::journal::write("key", serde_json::json!({ "provider": provider, "saved": true }));
    Ok("It works. Saved, encrypted by Windows. She uses it from her next start.".into())
}

/// Her online brain from the key saved for `provider` (any, free or paid): the models to use, picked from what the
/// key can reach, each tried once first.
#[tauri::command]
pub async fn online_brain(provider: String, model: Option<String>) -> Result<serde_json::Value, String> {
    // Your own plan needs no key: its app is signed in.
    let key = if providers::plan_kind(&provider).is_some() || provider == "ollamacloud" { Some(String::new()) } else { saved_key(&provider) };
    let Some(key) = key else { return Err("no key yet".into()) };
    let (fast, eyes) = providers::online_brain(&provider, &key, model.as_deref()).await.map_err(|e| format!("{e:#}"))?;
    capri_core::journal::write("online brain", serde_json::json!({ "provider": provider, "fast": fast, "eyes": eyes }));
    Ok(serde_json::json!({ "fast": fast, "eyes": eyes }))
}

/// A provider's key: from the environment, else the one saved in Windows' Credential Manager.
fn saved_key(provider: &str) -> Option<String> {
    let env = providers::key_env(provider).and_then(|v| std::env::var(v).ok()).filter(|k| !k.trim().is_empty());
    #[cfg(windows)]
    let key = env.or_else(|| capri_hands::platform::vault::key_get(provider));
    #[cfg(not(windows))]
    let key = env;
    key
}

/// The models you can pick for an online brain (a plan's, or what your key can use), recommended first.
#[tauri::command]
pub async fn plan_models(provider: String) -> Vec<providers::plan::PlanModel> {
    match (providers::plan_kind(&provider), saved_key(&provider)) {
        (Some(kind), _) => providers::plan::models(kind).await,
        (None, Some(key)) => providers::key_models(&provider, &key).await,
        (None, None) => Vec::new(),
    }
}

/// Opens the page where a key for `provider` is made, in your browser.
#[tauri::command]
pub fn key_page(provider: String) -> Result<(), String> {
    let url = providers::key_page(&provider).ok_or("no key page for that one")?;
    std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Forgets a saved key.
#[tauri::command]
pub fn delete_key(provider: String) {
    #[cfg(windows)]
    capri_hands::platform::vault::key_delete(&provider);
    capri_core::journal::write("key", serde_json::json!({ "provider": provider, "saved": false }));
}

/// Her bond as it stands (her settings window shows it).
#[tauri::command]
pub fn bond() -> serde_json::Value {
    let b = capri_core::bond::load();
    serde_json::json!({ "stage": b.stage, "name": capri_core::bond::STAGES[(b.stage as usize).min(4)].0, "points": b.points(), "days": b.days })
}

/// Her mood, from how she's treated on screen (thrown around, patted…): it colours her words for `minutes`.
#[tauri::command]
pub fn set_mood(state: tauri::State<'_, Mind>, mood: String, why: String, minutes: u64) {
    capri_core::journal::write("mood", serde_json::json!({ "mood": mood, "why": why, "minutes": minutes }));
    state.brain.set_mood(&mood, &why, minutes.min(60));
}

/// A typed request. Her answer comes as `capri://answer` (or "Stopped." after Esc); this returns once she's done.
#[tauri::command]
pub async fn ask(app: AppHandle, text: String) -> Result<bool, String> {
    if answer_confirm(&app, &text) {
        return Ok(true);
    }
    let done = crate::listen::start_job(&app, &text, false);
    while !done.load(std::sync::atomic::Ordering::Relaxed) {
        if !crate::listen::still_on(&app, &done) {
            // Stopped (Esc, her name) or replaced by a newer request.
            return Ok(true);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Ok(true)
}

/// "Forget our conversations" (settings): what she keeps of the conversation, and with `logs` her journal and model
/// log too (files in use are left). Returns how many files went.
#[tauri::command]
pub fn forget(state: tauri::State<'_, Mind>, logs: bool) -> usize {
    state.brain.forget();
    let mut gone = 0;
    if logs && let Ok(dir) = std::fs::read_dir(capri_core::journal::dir()) {
        for f in dir.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            let ours = ["journal-", "llm-", "capricorn-"].iter().any(|p| name.starts_with(p)) || name.ends_with(".wav");
            if ours && std::fs::remove_file(f.path()).is_ok() {
                gone += 1;
            }
        }
    }
    capri_core::journal::write("forgot", serde_json::json!({ "logs": logs, "files": gone }));
    gone
}

/// "Yes" / "no", typed or said, while she's asking whether to do a risky step: that answers it (the buttons weren't
/// the only way; a "yes" said started a new request and the question timed out as a no). True if it was an answer.
pub fn answer_confirm(app: &AppHandle, text: &str) -> bool {
    let Some(yes) = capri_brain::router::yes_no(text) else { return false };
    let mind = app.state::<Mind>();
    let newest = mind.pending.lock().unwrap().keys().max().copied();
    let Some(id) = newest else { return false };
    if let Some(tx) = mind.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(yes);
    }
    capri_core::journal::write("confirm answered", serde_json::json!({ "text": text, "yes": yes }));
    // Answered while she was still asking: she stops asking.
    crate::speak::stop(app, "answered");
    let _ = app.emit("capri://confirmed", id);
    true
}

#[tauri::command]
pub fn confirm(state: tauri::State<'_, Mind>, id: u64, yes: bool) {
    if let Some(tx) = state.pending.lock().unwrap().remove(&id) {
        let _ = tx.send(yes);
    }
}
