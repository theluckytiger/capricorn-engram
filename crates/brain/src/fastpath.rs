//! Requests her own code does, no AI in between: only when the *whole* sentence is one of these, nothing left over.
//! "pause" is the fast path; "pause and open discord" or "play bohemian rhapsody" are not, they go to her brain.
//! A fast-path call that fails (an app name that isn't sure, say) falls through to her brain too.

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct Fast {
    pub tool: &'static str,
    pub args: Value,
    /// How she answers: a line from her bank (instant) or a sentence written from the result (info questions).
    pub speak: Speak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speak {
    Line,
    Summarize,
}

/// Lowercase, no punctuation, without her name, politeness or filler around the request.
pub fn normalize(text: &str) -> String {
    // Curly apostrophes first ("what’s playing"), before everything else that isn't a letter becomes a space.
    let mut s: String = text.to_lowercase().replace('\u{2019}', "'").chars().map(|c| if c.is_alphanumeric() || c == '%' || c == '\'' { c } else { ' ' }).collect();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    const LEAD: &[&str] = &["hey ", "ok ", "okay ", "yo ", "capricorn ", "capri ", "please ", "can you ", "could you ", "would you ", "can u ", "pls ", "plz ", "just ", "go ahead and ", "and "];
    const TAIL: &[&str] = &[" please", " pls", " plz", " for me", " now", " capri", " capricorn", " thanks", " thank you"];
    loop {
        let before = s.clone();
        for p in LEAD {
            if let Some(rest) = s.strip_prefix(p) {
                s = rest.to_string();
            }
        }
        for p in TAIL {
            if let Some(rest) = s.strip_suffix(p) {
                s = rest.to_string();
            }
        }
        if s == before {
            break;
        }
    }
    s.trim().to_string()
}

fn words(s: &str) -> Vec<&str> {
    s.split(' ').collect()
}

/// `s` is exactly `verb` + one of `objects` (or `verb` alone when `alone` is allowed).
fn verb_object(s: &str, verbs: &[&str], objects: &[&str], alone: bool) -> bool {
    for v in verbs {
        if s == *v {
            return alone;
        }
        if let Some(rest) = s.strip_prefix(&format!("{v} "))
            && objects.contains(&rest)
        {
            return true;
        }
    }
    false
}

const MEDIA_OBJECTS: &[&str] = &["it", "this", "that", "music", "the music", "my music", "the song", "this song", "the video", "this video", "the track", "playback", "the playback", "youtube", "spotify", "the youtube video"];

pub fn parse(text: &str) -> Option<Fast> {
    let s = normalize(text);
    if s.is_empty() {
        return None;
    }
    let tool = |tool: &'static str, args: Value| Some(Fast { tool, args, speak: Speak::Line });
    let info = |tool: &'static str, args: Value| Some(Fast { tool, args, speak: Speak::Summarize });

    // media
    if verb_object(&s, &["pause", "stop"], MEDIA_OBJECTS, true) {
        return tool("media", json!({ "action": "pause" }));
    }
    if verb_object(&s, &["play", "resume", "unpause", "continue"], MEDIA_OBJECTS, true) {
        return tool("media", json!({ "action": "play" }));
    }
    if verb_object(&s, &["next", "skip"], &["song", "track", "video", "one", "this", "this song", "this one", "it", "the song"], true) || s == "next song" || s == "play the next song" {
        return tool("media", json!({ "action": "next" }));
    }
    if verb_object(&s, &["previous", "prev"], &["song", "track", "video", "one"], true) || matches!(s.as_str(), "last song" | "go back a song" | "play the previous song" | "previous song" | "the song before") {
        return tool("media", json!({ "action": "previous" }));
    }
    if matches!(
        s.as_str(),
        "what's playing" | "whats playing" | "what is playing" | "what's playing right now" | "what song is this" | "what's this song" | "whats this song" | "what is this song" | "what am i listening to" | "what's this song called" | "whats this song called" | "what is this song called" | "what's the name of this song" | "what's the song called" | "who sings this" | "who's singing" | "who is singing" | "who sings this song" | "who's this" | "name this song" | "what's this track" | "who made this song"
    ) {
        return info("media", json!({ "action": "status" }));
    }

    // screenshot
    if matches!(s.as_str(), "take a screenshot" | "screenshot" | "screenshot this" | "take a screenshot of this" | "screenshot my screen" | "take a screenshot of my screen" | "grab a screenshot" | "snap the screen" | "take a screen shot") {
        return tool("screenshot", json!({}));
    }

    // volume
    if let Some(n) = volume_to(&s) {
        return tool("volume", json!({ "action": "set", "percent": n }));
    }
    if matches!(s.as_str(), "louder" | "turn it up" | "volume up" | "turn the volume up" | "turn up the volume" | "turn up the sound" | "raise the volume") {
        return tool("volume", json!({ "action": "up" }));
    }
    if matches!(s.as_str(), "quieter" | "turn it down" | "volume down" | "turn the volume down" | "turn down the volume" | "turn down the sound" | "lower the volume") {
        return tool("volume", json!({ "action": "down" }));
    }
    if let Some(by) = volume_by(&s) {
        return tool("volume", json!({ "action": by.0, "by": by.1 }));
    }
    if matches!(s.as_str(), "max the volume" | "max volume" | "volume max" | "full volume" | "max out the volume" | "volume to max" | "volume to the max") {
        return tool("volume", json!({ "action": "set", "percent": 100 }));
    }
    if verb_object(&s, &["mute"], &["the sound", "sound", "the volume", "volume", "the audio", "audio", "the speakers", "speakers", "my speakers", "the pc", "my pc", "the computer"], false) {
        return tool("volume", json!({ "action": "mute" }));
    }
    if verb_object(&s, &["unmute"], &["the sound", "sound", "the volume", "volume", "the audio", "audio", "the speakers", "speakers", "my speakers", "the pc", "my pc", "the computer"], false) {
        return tool("volume", json!({ "action": "unmute" }));
    }
    if matches!(s.as_str(), "what's the volume" | "whats the volume" | "what is the volume" | "how loud is it") {
        return info("volume", json!({ "action": "get" }));
    }

    // browser tabs (never the window)
    let tab = match s.as_str() {
        "close this tab" | "close the tab" | "close tab" | "close that tab" | "kill this tab" => Some("close"),
        "reopen the tab" | "reopen that tab" | "reopen the tab i just closed" | "reopen the last tab" | "open the tab i just closed" | "bring back that tab" | "undo close tab" => Some("reopen"),
        "new tab" | "open a new tab" | "open new tab" => Some("new"),
        "next tab" => Some("next"),
        "previous tab" | "last tab" => Some("previous"),
        "go back" | "go back a page" | "back a page" | "previous page" => Some("back"),
        "go forward" | "forward a page" | "next page" => Some("forward"),
        "reload" | "refresh" | "reload the page" | "refresh the page" | "reload this page" | "refresh this page" => Some("reload"),
        _ => None,
    };
    if let Some(action) = tab {
        return tool("tab", json!({ "action": action }));
    }

    // Discord voice
    if matches!(s.as_str(), "mute my mic" | "mute me" | "mute my microphone" | "mute mic" | "mute discord" | "mute me on discord" | "mute my mic on discord" | "mute my discord mic" | "mute my discord") {
        return tool("discord", json!({ "action": "mute" }));
    }
    if matches!(s.as_str(), "unmute my mic" | "unmute me" | "unmute my microphone" | "unmute mic" | "unmute discord" | "unmute me on discord" | "unmute my mic on discord" | "unmute my discord mic" | "unmute my discord") {
        return tool("discord", json!({ "action": "unmute" }));
    }
    if matches!(s.as_str(), "deafen" | "deafen me" | "deafen me on discord" | "deafen discord") {
        return tool("discord", json!({ "action": "deafen" }));
    }
    if matches!(s.as_str(), "undeafen" | "undeafen me" | "undeafen me on discord" | "undeafen discord") {
        return tool("discord", json!({ "action": "undeafen" }));
    }
    if matches!(s.as_str(), "am i muted" | "is my mic muted" | "is my mic on" | "am i muted on discord") {
        return info("discord", json!({ "action": "status" }));
    }

    // windows
    if matches!(
        s.as_str(),
        "minimize all" | "minimise all" | "minimize everything" | "minimise everything" | "minimize all windows" | "minimise all windows" | "minimize all my windows" | "minimise all my windows" | "minimize my windows" | "minimise my windows" | "show desktop" | "show me the desktop" | "show the desktop"
    ) {
        return tool("windows", json!({ "action": "minimize_all" }));
    }
    if matches!(s.as_str(), "restore all" | "restore everything" | "restore my windows" | "restore all windows" | "restore all my windows" | "bring them back" | "bring back my windows" | "bring my windows back" | "bring everything back" | "unminimize everything" | "unminimize all") {
        return tool("windows", json!({ "action": "restore_all" }));
    }

    // open <one thing>: only a short plain name, and the open tool itself refuses anything it isn't sure of.
    for v in ["open up ", "open ", "launch ", "start up ", "start "] {
        if let Some(name) = s.strip_prefix(v) {
            let w = words(name);
            let padded = format!(" {name} ");
            let compound = [" and ", " then ", " on ", " in ", " with ", " to ", " from ", " for "].iter().any(|c| padded.contains(c));
            // "the third video", "his channel", "my last download": things on a page or in a folder, not an app.
            let vague = ["the ", "a ", "an ", "that ", "this ", "his ", "her ", "their ", "my last", "my latest", "my newest"].iter().any(|p| name.starts_with(p)) && name != "the recycle bin";
            let page = ["video", "channel", "tab", "link", "page", "song", "playlist", "first", "second", "third", "last"].iter().any(|x| w.contains(x));
            if !name.is_empty() && w.len() <= 4 && !compound && !vague && !page {
                return tool("open", json!({ "name": name }));
            }
            break;
        }
    }

    // weather now (questions about when or where go to her brain, which reads the forecast)
    if matches!(s.as_str(), "weather" | "what's the weather" | "whats the weather" | "what is the weather" | "what's the weather like" | "whats the weather like" | "how's the weather" | "hows the weather" | "what's the weather today" | "whats the weather today" | "weather today" | "what's the weather like today" | "whats the weather like today" | "what's the weather like outside" | "how's the weather today" | "hows the weather today" | "what's it like outside" | "whats it like outside") {
        return info("weather", json!({}));
    }
    None
}

fn volume_to(s: &str) -> Option<u32> {
    let rest = s
        .strip_prefix("set the volume to ")
        .or_else(|| s.strip_prefix("set volume to "))
        .or_else(|| s.strip_prefix("volume to "))
        .or_else(|| s.strip_prefix("volume at "))
        .or_else(|| s.strip_prefix("set the volume at "))
        .or_else(|| s.strip_prefix("turn the volume to "))
        .or_else(|| s.strip_prefix("put the volume at "))
        .or_else(|| s.strip_prefix("put the volume to "))
        .or_else(|| s.strip_prefix("volume "))?;
    let rest = rest.trim_end_matches(" percent").trim_end_matches('%').trim();
    rest.parse::<u32>().ok().filter(|n| *n <= 100)
}

fn volume_by(s: &str) -> Option<(&'static str, u32)> {
    for (prefix, dir) in [("turn it up by ", "up"), ("turn the volume up by ", "up"), ("volume up by ", "up"), ("turn it down by ", "down"), ("turn the volume down by ", "down"), ("volume down by ", "down")] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let n = rest.trim_end_matches(" percent").trim_end_matches('%').trim().parse::<u32>().ok()?;
            return Some((dir, n.min(100)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Option<(&'static str, Value)> {
        parse(s).map(|f| (f.tool, f.args))
    }

    #[test]
    fn whole_sentences_only() {
        assert_eq!(t("Capri, pause.").unwrap().1["action"], "pause");
        assert_eq!(t("can you pause the video please").unwrap().1["action"], "pause");
        assert!(t("pause and open discord").is_none());
        assert!(t("play bohemian rhapsody").is_none());
        assert!(t("stop talking").is_none());
        assert!(t("mute").is_none(), "plain mute could mean the mic");
        assert_eq!(t("mute my mic").unwrap().1["action"], "mute");
        // Curly apostrophes, as phones and some keyboards type them.
        assert_eq!(t("What’s playing?").unwrap().0, "media");
        assert_eq!(t("Unmute my Discord mic.").unwrap().1["action"], "unmute");
    }

    #[test]
    fn volume() {
        assert_eq!(t("volume 30").unwrap().1["percent"], 30);
        assert_eq!(t("set the volume to 45%").unwrap().1["percent"], 45);
        assert_eq!(t("turn it up by 20").unwrap().1["by"], 20);
        assert_eq!(t("max the volume").unwrap().1["percent"], 100);
        assert!(t("volume 300").is_none());
    }

    #[test]
    fn open() {
        assert_eq!(t("open discord").unwrap().1["name"], "discord");
        assert_eq!(t("hey capri open my downloads").unwrap().1["name"], "my downloads");
        assert!(t("open discord and spotify").is_none());
        assert!(t("open the third video").is_none());
        assert!(t("open his channel").is_none());
        assert!(t("open youtube in brave").is_none());
        assert!(t("start a timer for 10 minutes").is_none());
    }

    #[test]
    fn screenshot() {
        assert_eq!(t("Capri, take a screenshot please").unwrap().0, "screenshot");
        assert!(t("take a screenshot of the game and send it to Discord").is_none());
    }

    #[test]
    fn info() {
        assert_eq!(parse("what's playing?").unwrap().speak, Speak::Summarize);
        assert_eq!(t("what's the weather").unwrap().0, "weather");
        assert_eq!(t("Capri. What's the weather like today?").unwrap().0, "weather");
        assert!(t("when is it going to stop raining at home").is_none());
    }
}
