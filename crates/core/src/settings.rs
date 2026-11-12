//! Settings on disk (`%APPDATA%\capricorn-engram\config\settings.json`). API keys are never here: they live in the OS keyring.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Every field has a default, so an old or partial file still loads (`#[serde(default)]` fills what's missing).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// What she calls you.
    pub name: Option<String>,
    /// Make her yours: how she is with you, things she remembers, lines of yours.
    pub yours: Yours,
    /// Blackwall mode: she stops asking before risky steps. Money, wiping data, Windows security and code from the
    /// internet still always ask.
    pub blackwall: bool,
    /// For weather and "at home": a place name she geocodes once.
    pub home: Option<String>,
    pub brain: Brain,
    pub ears: Ears,
    pub voice: Voice,
    pub screens: Screens,
    pub privacy: Privacy,
    /// What she does during each game she's seen, by exe name (asked the first time).
    pub games: std::collections::BTreeMap<String, GameMode>,
    /// Her checking in on you: water, a stretch, food, bed, a break from a long game.
    pub care: Care,
    /// Her keys, Ctrl+Alt+letter, by what they do (`talk`, `wake_word`, `type`, `away`, `mute`, `ride`); missing ones
    /// keep their usual letter.
    pub keys: std::collections::BTreeMap<String, char>,
    /// Messaging her from Discord when you're away from the PC.
    pub phone: Phone,
}

/// Settings › Phone. Her bot's token isn't here: it's in the OS keyring, like her API keys.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Phone {
    /// She reads your messages to her bot and answers them. Off until you turn it on.
    pub on: bool,
    /// Your Discord user ID: the only person she takes messages from.
    pub discord_user: String,
}

/// Make her yours (Settings › Make her yours, or by voice: "call me…", "remember that…").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Yours {
    /// How she is with you, in your words ("meaner", "calls everything a gig").
    pub style: String,
    /// Things she remembers about you, in your words.
    pub memories: Vec<String>,
    /// Lines of yours she drops now and then.
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
    /// Ears closed too, until the game ends.
    Quiet,
    /// "Capri" still works; only the cloud thinks for her (her GPU model stays unloaded).
    Reachable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ears {
    /// Listening for her name ("Capri", "Capricorn", "hey Capri"…), on this PC, nothing recorded. Off: only the talk
    /// key. (`always` is its old name in the settings file.)
    #[serde(alias = "always")]
    pub wake_word: bool,
    /// Conversation mode: after a spoken answer, how many seconds she keeps listening for a follow-up without her
    /// name (again after every answer, until you stop talking). 0: she always waits for her name again.
    pub follow_up_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Voice {
    /// She says her answers out loud.
    pub enabled: bool,
    /// `supertonic`, `kokoro` or `kitten`, and the speaker number in it.
    pub engine: String,
    pub speaker: i32,
    pub speed: f32,
    /// Her sound effects (thrown, landing, a hit…), not her voice.
    pub sounds: bool,
}

impl Default for Voice {
    fn default() -> Self {
        // Her default voice: Kokoro jf_alpha, speaker 37.
        Self { enabled: true, engine: "kokoro".into(), speaker: 37, speed: 1.0, sounds: true }
    }
}

impl Default for Ears {
    fn default() -> Self {
        Self { wake_word: true, follow_up_secs: 8 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Brain {
    /// May anything go to a cloud model at all.
    pub cloud: bool,
    /// The model for each role, as `provider/model` (e.g. `ollama/qwen3-vl:8b-instruct`, `anthropic/claude-sonnet-5-5`).
    pub fast: String,
    pub smart: Option<String>,
    pub eyes: Option<String>,
    /// Cloud requests allowed per day before `smart` falls back to local.
    pub daily_cloud_requests: u32,
    /// Her local model picked for this PC at every start (its graphics card's memory, else its memory); off once you
    /// choose one yourself.
    pub auto: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Screens {
    /// She follows your pointer to another screen after a few seconds there. Off by default: her jumping to whatever
    /// screen you're working on gets in the way.
    pub follow_pointer: bool,
    /// Pin her to one screen (its index); `None` lets her move.
    pub locked_to: Option<usize>,
    /// How big she is: 1 is her usual size (0.5 tiny … 1.8 giant).
    pub size: f32,
}

/// Her check-ins (minutes between them; 0 turns one off).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Care {
    pub on: bool,
    pub water: u32,
    pub stretch: u32,
    /// Your eyes off the screen (off unless set).
    pub eyes: u32,
    /// Food when you've been at the PC three hours into lunch or dinner time.
    pub food: bool,
    /// From this hour until six, bed (once a night); 24 turns it off.
    pub bed_hour: u32,
    /// A break from the same game after this long.
    pub game_break: u32,
}

impl Default for Care {
    fn default() -> Self {
        Self { on: true, water: 75, stretch: 100, eyes: 0, food: true, bed_hour: 2, game_break: 120 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Privacy {
    /// Screenshots may be sent to a cloud model (off: they only ever go to a local one).
    pub screenshots_to_cloud: bool,
    /// She speaks out loud while you're in a call.
    pub speak_in_calls: bool,
    /// She watches coding AIs at work (Claude Code's transcripts, read on this PC) and reacts to what they do.
    pub watch_coders: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self { screenshots_to_cloud: false, speak_in_calls: false, watch_coders: true }
    }
}

impl Default for Brain {
    fn default() -> Self {
        Self {
            cloud: false,
            auto: true,
            fast: "ollama/qwen3-vl:8b-instruct".into(),
            smart: None,
            eyes: Some("ollama/qwen3-vl:8b-instruct".into()),
            daily_cloud_requests: 200,
        }
    }
}

impl Default for Screens {
    fn default() -> Self {
        Self { follow_pointer: false, locked_to: None, size: 1.0 }
    }
}

/// Her data folder: `%APPDATA%\capricorn-engram\config` on Windows, `~/.config/capricorn-engram` on Linux. Not
/// `%APPDATA%\Capricorn`: that's the old Capricorn's, and uninstalling it may empty that folder.
pub fn data_dir() -> Result<PathBuf> {
    // Tests run in a folder of their own, so a test run never touches real settings.
    if let Some(d) = std::env::var_os("CAPRI_DATA_DIR") {
        return Ok(PathBuf::from(d));
    }
    let dirs = directories::ProjectDirs::from("", "", "capricorn-engram").context("no home folder")?;
    Ok(dirs.config_dir().to_path_buf())
}

impl Settings {
    pub fn path() -> Result<PathBuf> {
        Ok(data_dir()?.join("settings.json"))
    }

    /// Loads the settings, or the defaults when there's no file yet. A broken file is kept aside, not overwritten.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(s) => Ok(s),
                Err(e) => {
                    tracing::warn!("settings unreadable ({e}), keeping a copy and starting from defaults");
                    let _ = std::fs::rename(&path, path.with_extension("broken.json"));
                    Ok(Self::default())
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("reading settings"),
        }
    }

    /// Loads them, changes them, saves them (her "call me…" and "remember that…").
    pub fn update(change: impl FnOnce(&mut Settings)) -> Result<Settings> {
        let mut s = Self::load()?;
        change(&mut s);
        s.save()?;
        Ok(s)
    }

    /// Writes through a temporary file so a crash mid-write never leaves half a file.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_keeps_defaults() {
        let s: Settings = serde_json::from_str(r#"{"name":"Max","brain":{"cloud":true}}"#).unwrap();
        assert_eq!(s.name.as_deref(), Some("Max"));
        assert!(s.brain.cloud);
        assert_eq!(s.brain.fast, Brain::default().fast);
        assert!(!s.screens.follow_pointer);
    }
}
