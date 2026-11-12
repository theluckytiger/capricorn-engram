//! Her ears inside the app: what they hear goes to her overlay (`capri://heard`), requests go to her brain, and
//! the talk key works as push-to-talk (hold) or "listen now" (tap).
//!
//! Her name always wins: said while she talks, she stops mid-word; said while she's still working on something you
//! asked, she keeps at it quietly: a new request replaces it, and if none comes its answer is given when she stops
//! listening. Either way she listens for what you want now, and "stop" / "never mind" then means just that: quiet,
//! nothing else (what she was working on is dropped).
//!
//! Conversation mode (Settings › Ears, on by default): after her spoken answer she listens on for a few seconds
//! without her name, again after every answer, until you stop talking.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use capri_ears::{Ears, Heard};
use tauri::{AppHandle, Emitter, Manager};

pub struct Listening {
    pub ears: Option<Ears>,
    pub wake_word: Mutex<bool>,
    /// Your mic muted for her (Ctrl+Alt+N): her name, the talk key and her follow-ups all off, the mic closed.
    pub muted: AtomicBool,
    /// Conversation mode: seconds she listens on after her answer (0: off).
    follow_up: AtomicU32,
    /// Her name came while she was still working: her answer waits until she stops listening (`held`), unless a new
    /// request replaces it.
    hold_answer: AtomicBool,
    held: Mutex<Option<(capri_brain::Answer, bool)>>,
    /// When the talk key went down, to tell a tap from a hold.
    talk_down: Mutex<Option<Instant>>,
    /// The spoken request she's working on, so her name can drop it.
    busy: Mutex<Option<Busy>>,
    /// When her name last cut her off (or dropped what she was doing): "stop" right after means "be quiet".
    interrupted: Mutex<Option<Instant>>,
    /// What you said while she worked on a job, for that job's next step.
    pub told: Mutex<Vec<String>>,
    /// When the job she's on last got anywhere (a step started or ended, her brain was asked).
    pub moved: Mutex<Instant>,
}

struct Busy {
    text: String,
    task: tauri::async_runtime::JoinHandle<()>,
    done: std::sync::Arc<AtomicBool>,
}

/// Where her speech models are: `CAPRI_MODELS`, next to the app (installed), or the repo's `models/` (dev).
pub fn models_dir(app: &AppHandle) -> PathBuf {
    crate::models::dir(app.path().resource_dir().ok())
}

pub fn start(app: &AppHandle, settings: &capri_core::Settings) -> Listening {
    let models = models_dir(app);
    let follow_up = settings.ears.follow_up_secs;
    let listening = |ears: Option<Ears>, wake_word: bool| Listening {
        ears,
        wake_word: Mutex::new(wake_word),
        muted: AtomicBool::new(false),
        follow_up: AtomicU32::new(follow_up),
        hold_answer: AtomicBool::new(false),
        held: Mutex::new(None),
        talk_down: Mutex::new(None),
        busy: Mutex::new(None),
        interrupted: Mutex::new(None),
        told: Mutex::new(Vec::new()),
        moved: Mutex::new(Instant::now()),
    };
    if !capri_ears::models_ok(&models) {
        tracing::warn!("no speech models in {}: her ears are off", models.display());
        return listening(None, false);
    }
    let handle = app.clone();
    // Dev: CAPRI_EARS_WAV=clip.wav (16 kHz) is heard instead of the mic, to test the spoken path without talking.
    let source = match std::env::var("CAPRI_EARS_WAV").ok().and_then(|f| sherpa_onnx::Wave::read(&f)) {
        Some(w) => capri_ears::engine::Source::Samples(w.samples().to_vec()),
        None => capri_ears::engine::Source::Mic,
    };
    // Dev builds keep what she heard (the last 60 sentences, on this PC), so a mishearing can be listened to.
    let keep_audio = cfg!(debug_assertions).then(|| capri_core::journal::dir().join("audio"));
    let config = capri_ears::Config { models, threads: 4, source, keep_audio };
    let ears = match capri_ears::start(config, move |heard| on_heard(&handle, heard)) {
        Ok(e) => Some(e),
        Err(e) => {
            tracing::error!("ears: {e:#}");
            None
        }
    };
    if let Some(e) = &ears {
        e.set_always(settings.ears.wake_word);
    }
    listening(ears, settings.ears.wake_word)
}

fn on_heard(app: &AppHandle, heard: Heard) {
    use capri_core::journal;
    use serde_json::json;
    match &heard {
        Heard::Utterance { text, ms, verdict, audio } => {
            tracing::debug!("heard ({ms} ms, {verdict}): {text}");
            journal::write("heard", json!({ "text": text, "ms": ms, "verdict": verdict, "audio": audio }));
        }
        Heard::Wake => {
            journal::write("wake", json!({}));
            interrupt(app, "her name");
        }
        Heard::BargeIn { text, ms } => {
            tracing::debug!("her name over her voice ({ms} ms): {text}");
            journal::write("barge_in", json!({ "heard": text, "ms": ms }));
            interrupt(app, "her name, said over her");
            // Her panel shows she's listening, like after her name alone.
            let _ = app.emit("capri://heard", Heard::Wake);
            return;
        }
        Heard::Timeout => {
            journal::write("timeout", json!({}));
            let _ = app.emit("capri://heard", &heard);
            // Nothing new came: the answer she held back is given now.
            let l = app.state::<Listening>();
            l.hold_answer.store(false, Ordering::Relaxed);
            let held = l.held.lock().unwrap().take();
            if let Some((answer, spoken)) = held {
                journal::write("held answer", json!({ "line": answer.line }));
                crate::brain::deliver(app, &answer, spoken);
            }
            return;
        }
        Heard::Mic { open, name } => journal::write("mic", json!({ "open": open, "name": name })),
        Heard::Error { message } => {
            tracing::warn!("ears: {message}");
            journal::write("error", json!({ "where": "ears", "message": message }));
        }
        Heard::Request { text: as_heard, after_name, .. } => {
            // What Parakeet is known to mishear ("my Discord Mike"), as meant.
            let fixed = capri_ears::wake::fix(as_heard);
            journal::write("heard_request", json!({ "text": fixed, "after_name": after_name, "as_heard": (fixed != *as_heard).then_some(as_heard) }));
            let text = &fixed;
            if *after_name {
                interrupt(app, "her name");
            }
            let l = app.state::<Listening>();
            let recently = l.interrupted.lock().unwrap().take().is_some_and(|at| at.elapsed() < Duration::from_secs(20));
            if recently && is_hush(text) {
                // "Capri… stop": she already stopped; that's all you wanted. What she was working on goes too.
                let old = l.busy.lock().unwrap().take();
                if let Some(old) = old {
                    drop_busy(old, "stop");
                }
                l.hold_answer.store(false, Ordering::Relaxed);
                l.held.lock().unwrap().take();
                journal::write("hushed", json!({ "text": text }));
                let _ = app.emit("capri://heard", json!({ "kind": "hushed" }));
                return;
            }
            // Her answer is said out loud; her ears reopen for your answer when she asked something (speak.rs).
            // The answer to her "do it?" about a risky step, said: not a new request.
            if crate::brain::answer_confirm(app, text) {
                return;
            }
            start_job(app, text, true);
        }
        Heard::Level { .. } | Heard::Voice { .. } => {}
    }
    let _ = app.emit("capri://heard", &heard);
}

/// One request at a time, spoken or typed, worked on in the background; her answer comes as `capri://answer`. While
/// she works on it, Esc (from any app) stops her after the step she's on. Done when the returned flag is set.
pub fn start_job(app: &AppHandle, text: &str, spoken: bool) -> std::sync::Arc<AtomicBool> {
    // Said while she works on a job: it goes into that job ("Got it."), not over it.
    if let Some(done) = tell_job(app, text) {
        return done;
    }
    // A new job starts with nothing said to it yet (words said to one stopped with Esc don't carry over).
    app.state::<Listening>().told.lock().unwrap().clear();
    let (app2, text2) = (app.clone(), text.to_string());
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let done2 = done.clone();
    let esc = tauri_plugin_global_shortcut::Shortcut::new(None, tauri_plugin_global_shortcut::Code::Escape);
    {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let _ = app.global_shortcut().register(esc);
    }
    let task = tauri::async_runtime::spawn(async move {
        let _ = crate::brain::answer(&app2, &text2, spoken).await;
        done2.store(true, Ordering::Relaxed);
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let _ = app2.global_shortcut().unregister(esc);
        // Told something as she was finishing: it's done now, as a request of its own.
        let left: Vec<String> = app2.state::<Listening>().told.lock().unwrap().drain(..).collect();
        // Said several times over while she was stuck: the last thing that asks for something, not all of it as one sentence.
        if let Some(next) = left.iter().rev().find(|t| capri_brain::router::asks_to_do(t)).or(left.last()) {
            if left.len() > 1 {
                capri_core::journal::write("held requests", serde_json::json!({ "all": left, "doing": next }));
            }
            start_job(&app2, next, spoken);
        }
    });
    let l = app.state::<Listening>();
    // A new request: an answer held back for one before it isn't wanted any more.
    l.hold_answer.store(false, Ordering::Relaxed);
    l.held.lock().unwrap().take();
    let old = l.busy.lock().unwrap().replace(Busy { text: text.to_string(), task, done: done.clone() });
    if let Some(old) = old {
        // One request at a time: two at once click over each other.
        drop_busy(old, "a new request");
    }
    done
}

/// You said something while she's working on a job: it's for that job (her next step reads it), and she says "Got
/// it." "Stop" stops the job instead. None when she isn't working on anything. (A new request used to replace the job
/// she was on: "no, the other one" threw the job away.)
fn tell_job(app: &AppHandle, text: &str) -> Option<std::sync::Arc<AtomicBool>> {
    let l = app.state::<Listening>();
    let done = l.busy.lock().unwrap().as_ref().filter(|b| !b.done.load(Ordering::Relaxed)).map(|b| b.done.clone())?;
    if is_hush(text) || matches!(text.trim().to_lowercase().trim_end_matches(['.', '!']), "cancel" | "never mind" | "nevermind" | "forget it") || ends_in_stop(text) {
        capri_core::journal::write("told mid-job", serde_json::json!({ "text": text, "stopped": true }));
        stop_job(app);
        return Some(done);
    }
    l.told.lock().unwrap().push(text.to_string());
    capri_core::journal::write("told mid-job", serde_json::json!({ "text": text }));
    // Her brain's been quiet a while: your words wait with the job, and saying "Got it" to each one sounded like nobody was home.
    let waiting = l.moved.lock().unwrap().elapsed() > Duration::from_secs(20);
    crate::speak::say(app, if waiting { "Still waiting on my brain. Say stop to drop it." } else { "Got it." }, false);
    Some(done)
}

/// Whether she's working on a request right now.
pub fn working(app: &AppHandle) -> bool {
    app.state::<Listening>().busy.lock().unwrap().as_ref().is_some_and(|b| !b.done.load(Ordering::Relaxed))
}

/// Whether the job `start_job` handed back is still hers to finish (not stopped or replaced).
pub fn still_on(app: &AppHandle, done: &std::sync::Arc<AtomicBool>) -> bool {
    app.state::<Listening>().busy.lock().unwrap().as_ref().is_some_and(|b| std::sync::Arc::ptr_eq(&b.done, done))
}

/// Esc while she works on a request: the request is dropped (the step under way finishes) and she's quiet.
pub fn stop_job(app: &AppHandle) {
    {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let _ = app.global_shortcut().unregister(tauri_plugin_global_shortcut::Shortcut::new(None, tauri_plugin_global_shortcut::Code::Escape));
    }
    let old = app.state::<Listening>().busy.lock().unwrap().take();
    if let Some(old) = old
        && drop_busy(old, "Esc")
    {
        crate::speak::stop(app, "Esc");
        let _ = app.emit("capri://answer", serde_json::json!({ "line": "Stopped.", "spoken": false, "steps": [] }));
    }
}

/// Her name (or the talk key) while she's talking or working on something: she stops talking. What she's working on
/// goes on quietly: her answer waits until she stops listening, and a new request replaces it. ("Capri", said to give
/// the next command, used to throw away the one she'd already done.)
fn interrupt(app: &AppHandle, why: &str) {
    let mut cut = crate::speak::stop(app, why);
    let l = app.state::<Listening>();
    if l.busy.lock().unwrap().as_ref().is_some_and(|b| !b.done.load(Ordering::Relaxed)) {
        l.hold_answer.store(true, Ordering::Relaxed);
        capri_core::journal::write("holding", serde_json::json!({ "why": why }));
        cut = true;
    }
    if cut {
        *l.interrupted.lock().unwrap() = Some(Instant::now());
    }
}

/// Drops a spoken request that's still being worked on: a step already under way finishes, nothing after it runs
/// and nothing is said. True if it was still going.
fn drop_busy(b: Busy, why: &str) -> bool {
    if b.done.load(Ordering::Relaxed) {
        return false;
    }
    b.task.abort();
    tracing::info!("dropped \"{}\": {why}", b.text);
    capri_core::journal::write("cancelled", serde_json::json!({ "text": b.text, "why": why }));
    true
}

/// "Stop", "shut up", "never mind"… said right after cutting her off: be quiet, do nothing.
/// Said to a job, ending in "stop" ("No, no, Capri. Can you find… stop."): the job stops. It went into the job as more
/// to do, and she searched the web for "Capri". Not "don't stop".
fn ends_in_stop(text: &str) -> bool {
    let s: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' }).collect();
    let words: Vec<&str> = s.split_whitespace().collect();
    let n = words.len();
    n >= 2 && matches!(words[n - 1], "stop" | "cancel") && !matches!(words[n - 2], "don't" | "dont" | "not" | "never" | "to")
}

fn is_hush(text: &str) -> bool {
    let s: String = text.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '\'').collect();
    let s = s.split_whitespace().filter(|w| !matches!(*w, "okay" | "ok" | "just" | "please" | "now" | "alright")).collect::<Vec<_>>().join(" ");
    matches!(
        s.as_str(),
        "" | "stop"
            | "stop it"
            | "stop stop"
            | "stop talking"
            | "shut up"
            | "be quiet"
            | "quiet"
            | "hush"
            | "shh"
            | "shush"
            | "enough"
            | "that's enough"
            | "thats enough"
            | "never mind"
            | "nevermind"
            | "forget it"
            | "cancel"
            | "cancel that"
            | "no"
            | "nothing"
            | "zip it"
            | "silence"
            | "got it"
            | "thanks"
            | "thank you"
            | "fine"
    )
}

impl Listening {
    /// Conversation mode, if it's on: after her answer she listens on without her name (her mic light blinks).
    pub fn follow_up(&self, app: &AppHandle, ears: &Ears) {
        let secs = self.follow_up.load(Ordering::Relaxed);
        if secs > 0 && !self.muted.load(Ordering::Relaxed) {
            ears.converse(Duration::from_secs(secs as u64));
            capri_core::journal::write("listening", serde_json::json!({ "why": "conversation" }));
            let _ = app.emit("capri://heard", serde_json::json!({ "kind": "follow_up" }));
        }
    }

    /// Conversation mode's seconds, changed in her settings (at once).
    pub fn set_follow_up(&self, secs: u32) {
        self.follow_up.store(secs, Ordering::Relaxed);
    }

    /// Her answer, if it's to wait (her name came while she worked on it): kept for when she stops listening.
    pub fn hold(&self, answer: &capri_brain::Answer, spoken: bool) -> bool {
        if !self.hold_answer.load(Ordering::Relaxed) {
            return false;
        }
        *self.held.lock().unwrap() = Some((answer.clone(), spoken));
        true
    }

    /// Whether she's listening for her name (the mic is open for it).
    pub fn wake_word_on(&self) -> bool {
        *self.wake_word.lock().unwrap() && !self.muted.load(Ordering::Relaxed)
    }
}

/// She asked you something: she listens for the answer without her name.
pub fn listen_for_answer(app: &AppHandle) {
    let l = app.state::<Listening>();
    if let Some(e) = &l.ears
        && l.wake_word_on()
    {
        e.attend(capri_ears::engine::LISTEN_FOR);
        e.answering(capri_ears::engine::LISTEN_FOR);
        capri_core::journal::write("listening", serde_json::json!({ "why": "she asked a question" }));
        let _ = app.emit("capri://heard", serde_json::json!({ "kind": "follow_up" }));
    }
}

/// Her "do this?" is up (`on`), or has been answered: a yes or a no is taken without her name until then.
pub fn waiting_for_answer(app: &AppHandle, on: bool) {
    if let Some(e) = &app.state::<Listening>().ears {
        e.answering(if on { Duration::from_secs(60) } else { Duration::ZERO });
    }
}

/// Her panel closed (✕, Esc): she stops listening for a request too.
pub fn stop_listening(app: &AppHandle) {
    if let Some(e) = &app.state::<Listening>().ears {
        e.stop_listening();
    }
}

/// The talk key: hold to talk, tap to have her listen for one request. Either way she stops talking first.
pub fn talk_key(app: &AppHandle, down: bool) {
    let l = app.state::<Listening>();
    if muted(app) {
        return;
    }
    let Some(ears) = &l.ears else {
        let _ = app.emit("capri://menu", "type");
        return;
    };
    let mut talk_down = l.talk_down.lock().unwrap();
    if down {
        *talk_down = Some(Instant::now());
        interrupt(app, "the talk key");
        ears.hold(true);
        let _ = app.emit("capri://heard", Heard::Wake);
    } else if let Some(at) = talk_down.take() {
        if at.elapsed() < Duration::from_millis(350) {
            // A tap: cancel the hold and listen for the next sentence instead.
            ears.attend(capri_ears::engine::LISTEN_FOR);
        } else {
            ears.hold(false);
        }
    }
}

/// "Talk to her" from her right-click menu.
#[tauri::command]
pub fn talk(app: AppHandle) {
    attend(&app);
}

/// "Talk to her" from her menu: listen for one request now (or open the box when she has no ears).
pub fn attend(app: &AppHandle) {
    let l = app.state::<Listening>();
    if muted(app) {
        return;
    }
    match &l.ears {
        Some(e) => {
            interrupt(app, "talk, from her menu");
            e.attend(capri_ears::engine::LISTEN_FOR);
            let _ = app.emit("capri://heard", Heard::Wake);
        }
        None => {
            let _ = app.emit("capri://menu", "type");
        }
    }
}

/// Ctrl+Alt+W: listening for her name on/off.
pub fn toggle_wake_word(app: &AppHandle) -> bool {
    let l = app.state::<Listening>();
    let mut on = l.wake_word.lock().unwrap();
    *on = !*on;
    if let Some(e) = &l.ears
        && !l.muted.load(Ordering::Relaxed)
    {
        e.set_always(*on);
    }
    capri_core::journal::write("wake_word", serde_json::json!({ "on": *on }));
    let _ = app.emit("capri://wake_word", *on);
    *on
}

/// Ctrl+Alt+N: your mic muted for her, or back. Muted, her ears close the mic (nothing could be listened to).
pub fn toggle_mute(app: &AppHandle) -> bool {
    let l = app.state::<Listening>();
    let now = !l.muted.load(Ordering::Relaxed);
    l.muted.store(now, Ordering::Relaxed);
    if let Some(e) = &l.ears {
        if now {
            e.set_always(false);
            e.stop_listening();
        } else {
            e.set_always(*l.wake_word.lock().unwrap());
        }
    }
    capri_core::journal::write("mute", serde_json::json!({ "on": now }));
    let _ = app.emit("capri://muted", now);
    now
}

/// Muted: says so (her panel) and true.
fn muted(app: &AppHandle) -> bool {
    let on = app.state::<Listening>().muted.load(Ordering::Relaxed);
    if on {
        let _ = app.emit("capri://muted", true);
    }
    on
}

#[cfg(test)]
mod tests {
    use super::is_hush;

    #[test]
    fn a_job_ending_in_stop_stops() {
        use super::ends_in_stop;
        assert!(ends_in_stop("No, no, Capri, Capri. Can you find Capri, stop."));
        assert!(ends_in_stop("okay cancel"));
        assert!(!ends_in_stop("don't stop"));
        assert!(!ends_in_stop("stop"));
        assert!(!ends_in_stop("find the bus stop near me"));
    }

    #[test]
    fn hush_words() {
        for s in ["Stop.", "stop talking", "Shut up!", "Okay, never mind.", "Be quiet", "That's enough.", "forget it"] {
            assert!(is_hush(s), "{s}");
        }
        for s in ["stop the music", "pause", "what's the weather", "close the tab"] {
            assert!(!is_hush(s), "{s}");
        }
    }
}
