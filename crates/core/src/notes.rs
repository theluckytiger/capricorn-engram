//! Her notes on how things worked in your apps ("in Stremio, the search box is at the top: click it, type, Enter"),
//! written when a job inside an app went right and read the next time one comes up for that app, so she's faster and
//! surer. On this PC only (`notes.json` next to her settings); you can read and wipe them in Settings › Memory.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// How many she keeps (the oldest go first).
const KEEP: usize = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    /// The app it's about ("Stremio", "Steam", "Brave Browser").
    pub app: String,
    /// What was asked, and what worked.
    pub how: String,
    /// When (local date).
    pub at: String,
}

fn path() -> Result<PathBuf> {
    Ok(crate::settings::Settings::path()?.with_file_name("notes.json"))
}

pub fn all() -> Vec<Note> {
    path().ok().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(notes: &[Note]) -> Result<()> {
    let p = path()?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(notes)?)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// A note on `app`; the same one again only moves it up.
pub fn add(app: &str, how: &str) -> Result<()> {
    let mut notes = all();
    notes.retain(|n| !(n.app.eq_ignore_ascii_case(app) && n.how == how));
    notes.push(Note { app: app.to_string(), how: how.to_string(), at: chrono::Local::now().format("%Y-%m-%d").to_string() });
    let extra = notes.len().saturating_sub(KEEP);
    notes.drain(..extra);
    save(&notes)
}

pub fn remove(index: usize) -> Result<()> {
    let mut notes = all();
    if index < notes.len() {
        notes.remove(index);
    }
    save(&notes)
}

pub fn clear() -> Result<()> {
    save(&[])
}

/// The newest notes (up to `n`) about the apps a request names, or the app in front.
pub fn relevant(request: &str, front_app: Option<&str>, n: usize) -> Vec<Note> {
    let ask = request.to_lowercase();
    let front = front_app.map(str::to_lowercase);
    let about = |app: &str| {
        let a = app.to_lowercase();
        // "Brave Browser" is named as "Brave", "Steam Client WebHelper" as "Steam".
        let first = a.split_whitespace().next().unwrap_or(&a).to_string();
        ask.contains(&a) || (first.len() >= 4 && ask.contains(&first)) || front.as_deref() == Some(a.as_str())
    };
    let mut out: Vec<Note> = all().into_iter().rev().filter(|n| about(&n.app)).take(n).collect();
    out.reverse();
    out
}
