//! Her reminders and timers, kept in `reminders.json` next to her settings so a restart doesn't lose them. Her
//! hands add and cancel them; the app watches for the ones coming due and says them.

use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: u64,
    pub due: DateTime<Local>,
    /// What to say ("check the oven"); a timer's is "".
    pub text: String,
}

fn path() -> Result<PathBuf> {
    Ok(crate::settings::data_dir()?.join("reminders.json"))
}

/// All of them, soonest first.
pub fn all() -> Vec<Reminder> {
    let mut list: Vec<Reminder> = path().ok().and_then(|p| crate::read_json_text(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    list.sort_by_key(|r| r.due);
    list
}

fn save(list: &[Reminder]) -> Result<()> {
    let p = path()?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(list)?)?;
    Ok(())
}

pub fn add(due: DateTime<Local>, text: &str) -> Result<Reminder> {
    let mut list = all();
    let id = list.iter().map(|r| r.id).max().unwrap_or(0) + 1;
    let r = Reminder { id, due, text: text.trim().to_string() };
    list.push(r.clone());
    save(&list)?;
    Ok(r)
}

/// Removes the ones `keep` says no to; returns them.
pub fn remove(keep: impl Fn(&Reminder) -> bool) -> Result<Vec<Reminder>> {
    let (kept, gone): (Vec<Reminder>, Vec<Reminder>) = all().into_iter().partition(|r| keep(r));
    if !gone.is_empty() {
        save(&kept)?;
    }
    Ok(gone)
}

/// The ones due by now, taken off the list.
pub fn take_due(now: DateTime<Local>) -> Vec<Reminder> {
    remove(|r| r.due > now).unwrap_or_default()
}
