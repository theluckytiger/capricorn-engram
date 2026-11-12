//! Her lines for fast-path results: instant, in her voice, and only saying what the tool saw.

use capri_hands::{Outcome, Status};
use serde_json::Value;

use crate::fastpath::Fast;

fn pick<'a>(options: &[&'a str]) -> &'a str {
    // Varied without a random crate: the clock's sub-second digits are random enough for a quip.
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as usize).unwrap_or(0);
    options[n / 1000 % options.len()]
}

/// The last number in a summary ("volume 32% → 50%" → 50).
fn last_number(s: &str) -> Option<u32> {
    s.split(|c: char| !c.is_ascii_digit()).rfind(|p| !p.is_empty())?.parse().ok()
}

pub fn for_fast(f: &Fast, out: &Outcome) -> String {
    if out.status == Status::Failed {
        return format!("{} {}.", pick(&["Nope.", "Didn't happen.", "No luck."]), first_sentence(&out.summary));
    }
    let action = f.args.get("action").and_then(Value::as_str).unwrap_or("");
    let unsure = out.status == Status::Unsure;
    let line = match (f.tool, action) {
        ("media", "pause") => pick(&["Paused.", "Frozen.", "Holding it right there."]).to_string(),
        ("media", "play") => pick(&["Playing.", "Back on.", "Rolling again."]).to_string(),
        ("media", "next") => pick(&["Skipped.", "Next one.", "Gone. Next."]).to_string(),
        ("media", "previous") => pick(&["Going back.", "Rewinding a track."]).to_string(),
        ("volume", "mute") if out.summary.contains("already") => "Already muted.".into(),
        ("volume", "unmute") if out.summary.contains("already") => "Sound's already on.".into(),
        ("volume", "mute") => pick(&["Muted.", "Silence."]).to_string(),
        ("volume", "unmute") => pick(&["Sound's back.", "Unmuted."]).to_string(),
        ("volume", _) => match last_number(&out.summary) {
            Some(n) => format!("{} {n}%.", pick(&["Volume", "Volume's at", "Set to"])),
            None => out.summary.clone(),
        },
        ("screenshot", _) => pick(&["Got it. Pictures, Screenshots. I'm not in it, don't worry.", "Snapped. It's in your Screenshots folder.", "Saved. Without me in it, sadly."]).to_string(),
        ("tab", "close") => pick(&["Tab's gone.", "Closed it."]).to_string(),
        ("tab", "reopen") => pick(&["It's back.", "Brought it back."]).to_string(),
        ("tab", "new") => "New tab.".into(),
        ("tab", "back") => "Back a page.".into(),
        ("tab", "forward") => "Forward.".into(),
        ("tab", "reload") => "Reloaded.".into(),
        ("tab", _) => "Switched tabs.".into(),
        ("discord", _) if out.summary.starts_with("already") => format!("Already that way: {}.", out.summary.trim_start_matches("already that way: ")),
        ("discord", "mute") => pick(&["Mic's muted.", "Muted. Nobody hears you now."]).to_string(),
        ("discord", "unmute") => pick(&["Mic's on.", "Unmuted. Talk."]).to_string(),
        ("discord", "deafen") => pick(&["Deafened.", "Headphones off. Peace."]).to_string(),
        ("discord", "undeafen") => pick(&["You can hear them again.", "Undeafened."]).to_string(),
        ("windows", "minimize_all") => match last_number(&out.summary) {
            Some(n) => format!("{} {n} windows down.", pick(&["Clean slate.", "Poof.", "Swept."])),
            None => "Minimized everything.".into(),
        },
        ("windows", "restore_all") => pick(&["They're back.", "Everything's back where it was."]).to_string(),
        ("open", _) if out.summary.contains("already open") => {
            let name = out.summary.split(" was already").next().unwrap_or("it");
            format!("{name} was already open. Brought it up.")
        }
        ("open", _) => {
            let name = f.args.get("name").and_then(Value::as_str).unwrap_or("it");
            format!("{} {}.", pick(&["Opening", "Pulling up", "Launching"]), title_case(name))
        }
        _ => out.summary.clone(),
    };
    if unsure { format!("{line} {}", first_sentence(&out.summary)) } else { line }
}

fn first_sentence(s: &str) -> String {
    let s = s.split(". ").next().unwrap_or(s).trim_end_matches('.');
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn title_case(s: &str) -> String {
    s.split(' ').map(first_sentence).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fastpath::parse;

    #[test]
    fn volume_line_uses_the_result() {
        let f = parse("volume 50").unwrap();
        let line = for_fast(&f, &Outcome::ok("volume 32% → 50%"));
        assert!(line.ends_with("50%."), "{line}");
    }

    #[test]
    fn failure_says_why() {
        let f = parse("open blorp").unwrap();
        let line = for_fast(&f, &Outcome::failed("no app called \"blorp\" in the Start menu"));
        assert!(line.contains("blorp"), "{line}");
    }
}
