//! Her bond with you, over weeks: days together, chats, jobs done, reminders kept and head pats add up through five
//! stages. Kept in `bond.json` next to her settings. Reaching a stage is a moment (the app shows it); the stage colours
//! how she talks to you.

use std::path::PathBuf;

use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bond {
    pub first_day: Option<NaiveDate>,
    pub last_day: Option<NaiveDate>,
    /// Days she's been up with you.
    pub days: u32,
    pub chats: u32,
    pub jobs: u32,
    pub reminders: u32,
    pub pats: u32,
    /// The stage she's shown you she reached (0..=4).
    pub stage: u8,
}

/// What added to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    Chat,
    Job,
    Reminder,
    Pat,
}

/// The five stages: their name, and how she is with you at each.
pub const STAGES: [(&str, &str); 5] = [
    ("stranger", "You've only just met them: curious, teasing, a little guarded, testing them."),
    ("acquaintance", "You know them a bit now: friendlier teasing, you give them a nickname now and then."),
    ("choom", "They're your choom: warm under the teasing, you look out for them, inside jokes."),
    ("partner in crime", "You're close: openly fond, protective, you miss them when they're gone and say so."),
    ("family", "They're yours and you're theirs: devoted, \"we\" comes easily, still a menace."),
];

/// Points to reach each stage.
const AT: [u32; 5] = [0, 25, 80, 200, 450];

impl Bond {
    pub fn points(&self) -> u32 {
        self.days * 3 + self.chats + self.jobs * 2 + self.reminders * 2 + self.pats / 2
    }

    /// The stage the points say (may be past the one she's shown).
    pub fn earned(&self) -> u8 {
        AT.iter().rposition(|&p| self.points() >= p).unwrap_or(0) as u8
    }
}

fn path() -> Result<PathBuf> {
    Ok(crate::settings::data_dir()?.join("bond.json"))
}

pub fn load() -> Bond {
    path().ok().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(b: &Bond) -> Result<()> {
    let p = path()?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(b)?)?;
    Ok(())
}

/// A day she's up (counted once per date), or a moment: added and saved. Returns the new stage when she just reached
/// one (to be shown), else `None`.
pub fn add(moment: Option<Moment>) -> Option<u8> {
    let mut b = load();
    let today = Local::now().date_naive();
    if b.last_day != Some(today) {
        b.days += 1;
        b.last_day = Some(today);
        b.first_day.get_or_insert(today);
    }
    match moment {
        Some(Moment::Chat) => b.chats += 1,
        Some(Moment::Job) => b.jobs += 1,
        Some(Moment::Reminder) => b.reminders += 1,
        Some(Moment::Pat) => b.pats += 1,
        None => {}
    }
    let earned = b.earned();
    let reached = (earned > b.stage).then_some(earned);
    if let Some(s) = reached {
        b.stage = s;
    }
    let _ = save(&b);
    reached
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages() {
        let mut b = Bond::default();
        assert_eq!(b.earned(), 0);
        b.days = 5;
        b.chats = 10;
        assert_eq!(b.points(), 25);
        assert_eq!(b.earned(), 1);
        b.jobs = 100;
        assert_eq!(b.earned(), 3);
    }
}
