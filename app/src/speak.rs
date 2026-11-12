//! Her voice inside the app: her answers out loud, her ears told what she's saying (so her own voice in the mic isn't
//! taken for yours, and her name said over her still cuts her off), listening for your answer when she asked you
//! something, and her loudness sent to her overlay (`capri://said`) for her mouth.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use capri_voice::{Engine, Said, Speaker, VoiceSpec};
use tauri::{AppHandle, Emitter, Manager};

pub struct Voice {
    pub speaker: Option<Speaker>,
    pub enabled: AtomicBool,
    /// The line she's saying (or about to), from `say` until it's finished.
    line: Mutex<Option<Line>>,
}

struct Line {
    id: u64,
    text: String,
    /// It answers something said out loud: when it's a question, she listens for your answer.
    spoken: bool,
}

fn spec(v: &capri_core::settings::Voice) -> VoiceSpec {
    let engine = match v.engine.as_str() {
        "kokoro" => Engine::Kokoro,
        "kitten" => Engine::Kitten,
        _ => Engine::Supertonic,
    };
    VoiceSpec { engine, speaker: v.speaker, speed: v.speed }
}

pub fn start(app: &AppHandle, settings: &capri_core::Settings, models: std::path::PathBuf) -> Voice {
    let spec = spec(&settings.voice);
    // Words she's learned to say, beside her settings.
    if let Ok(dir) = capri_core::settings::data_dir() {
        capri_voice::engine::set_learned(dir.join("words.txt"));
    }
    let speaker = if capri_voice::engine::installed(&models, spec.engine) {
        let handle = app.clone();
        match capri_voice::start(models, spec.clone(), move |said| on_said(&handle, said)) {
            Ok(s) => {
                tracing::info!("voice: {:?} speaker {}", spec.engine, spec.speaker);
                Some(s)
            }
            Err(e) => {
                tracing::error!("voice: {e:#}");
                None
            }
        }
    } else {
        tracing::warn!("no {:?} voice model in {}: she'll only write", spec.engine, models.display());
        None
    };
    Voice { speaker, enabled: AtomicBool::new(settings.voice.enabled), line: Mutex::new(None) }
}

fn on_said(app: &AppHandle, said: Said) {
    let listening = app.state::<crate::listen::Listening>();
    let voice = app.state::<Voice>();
    match &said {
        Said::Started { id, read_as } => {
            tracing::debug!("saying line {id}");
            let text = voice.line.lock().unwrap().as_ref().filter(|l| l.id == *id).map(|l| l.text.clone());
            // How her voice read it, when that isn't the line as written ("04:00" → "four").
            let read_as = Some(read_as).filter(|r| text.as_deref() != Some(r.as_str()));
            capri_core::journal::write("voice", serde_json::json!({ "line": id, "started": true, "read_as": read_as }));
            if let (Some(e), Some(text)) = (&listening.ears, text) {
                e.speaking(Some(&text));
            }
        }
        Said::Finished { id, cut } => {
            tracing::debug!("line {id} {}", if *cut { "cut off" } else { "said" });
            capri_core::journal::write("voice", serde_json::json!({ "line": id, "finished": true, "cut": cut }));
            let line = {
                let mut l = voice.line.lock().unwrap();
                if l.as_ref().is_some_and(|l| l.id == *id) { l.take() } else { None }
            };
            if let Some(e) = &listening.ears {
                e.speaking(None);
                if let Some(line) = line.filter(|l| l.spoken && !*cut) {
                    if line.text.trim_end().ends_with('?') {
                        crate::listen::listen_for_answer(app);
                    } else {
                        listening.follow_up(app, e);
                    }
                }
            }
        }
        Said::Error { message } => {
            tracing::warn!("voice: {message}");
            capri_core::journal::write("error", serde_json::json!({ "where": "voice", "message": message }));
        }
        Said::Level { .. } => {}
    }
    let _ = app.emit("capri://said", &said);
}

/// Says a line if her voice is on. `spoken`: it answers something said out loud.
/// Returns false if she didn't say it (no voice, or voice off): the caller handles listening for an answer then.
pub fn say(app: &AppHandle, line: &str, spoken: bool) -> bool {
    let voice = app.state::<Voice>();
    let Some(sp) = &voice.speaker else { return false };
    if !voice.enabled.load(Ordering::Relaxed) || line.trim().is_empty() {
        return false;
    }
    stop(app, "a new line");
    let id = sp.say(line);
    capri_core::journal::write("voice", serde_json::json!({ "line": id, "text": line }));
    *voice.line.lock().unwrap() = Some(Line { id, text: line.to_string(), spoken });
    true
}

/// Stops her mid-word, if she's talking (`why` goes in her journal). True if she was.
pub fn stop(app: &AppHandle, why: &str) -> bool {
    let voice = app.state::<Voice>();
    let Some(sp) = &voice.speaker else { return false };
    let Some(id) = voice.line.lock().unwrap().as_ref().map(|l| l.id) else { return false };
    tracing::debug!("stopping line {id}: {why}");
    capri_core::journal::write("hush", serde_json::json!({ "line": id, "why": why }));
    sp.stop();
    true
}

/// Her panel closed (✕, Esc, or by itself): she stops talking. Only you closing it (✕, Esc) stops her listening for
/// a request too; her panel timing out once cut her ears off mid-sentence.
#[tauri::command]
pub fn hush(app: AppHandle, why: Option<String>) {
    let why = why.as_deref().unwrap_or("her panel closed");
    stop(&app, why);
    if matches!(why, "✕ clicked" | "Esc") {
        crate::listen::stop_listening(&app);
    }
}
