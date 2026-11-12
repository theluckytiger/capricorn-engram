//! Discord's own mic and headphone switches (bottom left of its window), by their real state: muting a muted mic
//! says so instead of unmuting it.
//!
//! Discord's panel can show a change late (a mute that worked read back as "still unmuted"), so after a press
//! it's read again for a moment, by the switch's state, by its new name ("Unmute") and by its place, and a change
//! that never shows is reported as unsure, never as failed.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::platform::{self, Control, Pressed};
use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg};

pub struct Discord;

/// One of your own switches as Discord shows it, and whether it's on.
#[derive(Debug, Clone)]
struct Switch {
    control: Control,
    on: Option<bool>,
}

/// Your own switch called `name` ("Mute"): the lowest one in the window (the user panel, bottom left; a call's
/// panel can have more with the same name). Discord may call it `renamed` ("Unmute") while it's on.
fn switch(cs: &[Control], name: &str, renamed: &str) -> Option<Switch> {
    let lowest = |want: &str, toggles: bool| cs.iter().filter(|c| c.name.eq_ignore_ascii_case(want) && (!toggles || c.toggled.is_some())).max_by_key(|c| c.rect[1]);
    if let Some(c) = lowest(name, true) {
        return Some(Switch { control: c.clone(), on: c.toggled });
    }
    lowest(renamed, false).map(|c| Switch { control: c.clone(), on: Some(true) })
}

fn mic(cs: &[Control]) -> Option<Switch> {
    switch(cs, "Mute", "Unmute")
}

fn ears(cs: &[Control]) -> Option<Switch> {
    switch(cs, "Deafen", "Undeafen")
}

/// Whether the switch `was` (read before the press) is on now: by its name again, or, if Discord renamed it, by the
/// control in the same place.
fn now_on(cs: &[Control], was: &Switch, which: fn(&[Control]) -> Option<Switch>) -> Option<bool> {
    which(cs).and_then(|s| s.on).or_else(|| cs.iter().find(|c| c.rect == was.control.rect).and_then(|c| c.toggled))
}

/// Every control that looks like a mic or headphone switch, as seen, for her journal (Discord's names and states are
/// what this tool goes by, and they've surprised it before).
fn switches(cs: &[Control]) -> Value {
    let list: Vec<Value> = cs
        .iter()
        .filter(|c| {
            let n = c.name.to_lowercase();
            n.contains("mute") || n.contains("deafen") || n.contains("microphone") || n.contains("headphone") || n.contains("camera") || n.contains("screen") || n.contains("stream")
        })
        .map(|c| json!({ "kind": c.kind, "name": c.name, "toggled": c.toggled, "top": c.rect[1], "left": c.rect[0] }))
        .collect();
    json!(list)
}

fn describe(mic: Option<bool>, ears: Option<bool>) -> String {
    let mic = match mic {
        Some(true) => "mic muted",
        Some(false) => "mic on",
        None => "mic unknown",
    };
    let ears = match ears {
        Some(true) => "deafened",
        Some(false) => "not deafened",
        None => "deafen unknown",
    };
    format!("{mic}, {ears}")
}

#[async_trait]
impl Tool for Discord {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "discord".into(),
            description: "Discord's voice controls: the user's own mic (\"mute me\", \"I need to mute\", \"can they hear me?\"): \
                          mute or unmute it, deafen or undeafen, or `status` for how they are \
                          now; in a call, camera_on / camera_off, share_screen (Discord then asks what to share) / \
                          stop_sharing. Reads the real state first and only changes what needs changing."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "action": { "type": "string", "enum": ["status", "mute", "unmute", "deafen", "undeafen", "camera_on", "camera_off", "share_screen", "stop_sharing"] } },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        match args.get("action").and_then(Value::as_str) {
            Some("status") => Risk::Read,
            // Your face or your screen going out to a call: asked first.
            Some("camera_on" | "share_screen") => Risk::Risky,
            _ => Risk::Normal,
        }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        let dry = ctx.dry_run;
        tokio::task::spawn_blocking(move || {
            let Some(w) = platform::list_windows().into_iter().find(|w| matches!(w.exe.as_str(), "discord" | "discordptb" | "discordcanary")) else {
                return Outcome::failed("Discord isn't open (or it's only in the tray)");
            };
            let cs = match platform::controls(w.id, 2000) {
                Ok(cs) => cs,
                Err(e) => return Outcome::failed(format!("couldn't read Discord: {e:#}")),
            };
            let now = describe(mic(&cs).and_then(|s| s.on), ears(&cs).and_then(|s| s.on));
            let mut data = json!({ "before": switches(&cs) });
            // The call's buttons: Discord names them by what they'll do, and renames them once done.
            let call = match action.as_str() {
                "camera_on" => Some(("Turn On Camera", "Turn Off Camera", "camera on")),
                "camera_off" => Some(("Turn Off Camera", "Turn On Camera", "camera off")),
                "share_screen" => Some(("Share Your Screen", "Stop Streaming", "screen share started (Discord asks what to share)")),
                "stop_sharing" => Some(("Stop Streaming", "Share Your Screen", "screen share stopped")),
                _ => None,
            };
            if let Some((press, after, done)) = call {
                let find = |cs: &[Control], n: &str| cs.iter().filter(|c| c.name.eq_ignore_ascii_case(n)).max_by_key(|c| c.rect[1]).cloned();
                if find(&cs, press).is_none() && find(&cs, after).is_some() {
                    return Outcome::ok(format!("already that way: {done}")).with(data);
                }
                let Some(b) = find(&cs, press) else {
                    return Outcome::failed(format!("no \"{press}\" button in Discord: are you in a voice call, with its window open?")).with(data);
                };
                if dry {
                    return Outcome::ok(format!("(dry run) would press \"{press}\" in Discord")).with(data);
                }
                if let Err(e) = platform::press_control(w.id, &b) {
                    return Outcome::failed(format!("couldn't press it: {e:#}")).with(data);
                }
                let deadline = Instant::now() + Duration::from_millis(2000);
                while Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(250));
                    let cs = platform::controls(w.id, 2000).unwrap_or_default();
                    data["after"] = switches(&cs);
                    if find(&cs, after).is_some() || (action == "share_screen" && find(&cs, press).is_none()) {
                        return Outcome::ok(format!("done: {done}")).with(data);
                    }
                }
                return Outcome::unsure(format!("pressed \"{press}\", but Discord hasn't shown the change yet")).with(data);
            }
            let (which, want, done): (fn(&[Control]) -> Option<Switch>, bool, &str) = match action.as_str() {
                "status" => return Outcome::ok(format!("Discord: {now}")).with(data),
                "mute" => (mic, true, "mic muted"),
                "unmute" => (mic, false, "mic unmuted"),
                "deafen" => (ears, true, "deafened"),
                "undeafen" => (ears, false, "undeafened"),
                other => return Outcome::failed(format!("unknown action \"{other}\"")),
            };
            let Some(was) = which(&cs) else {
                return Outcome::failed("couldn't find Discord's mic and headphone buttons (is its window showing the user panel?)").with(data);
            };
            if was.on == Some(want) {
                return Outcome::ok(format!("already that way: {now}")).with(data);
            }
            if dry {
                return Outcome::ok(format!("(dry run) would press {} \"{}\" in Discord", was.control.kind, was.control.name)).with(data);
            }
            let pressed = platform::press_control(w.id, &was.control);
            data["pressed"] = json!(format!("{:?}", pressed.as_ref().map_err(|e| format!("{e:#}"))));
            if let Err(e) = &pressed {
                return Outcome::failed(format!("couldn't press it: {e:#}")).with(data);
            }
            // Read Discord again for a moment: its panel can lag behind the press.
            let deadline = Instant::now() + Duration::from_millis(1500);
            let mut seen = matches!(pressed, Ok(Pressed::Toggled(s)) if s == want).then_some(want);
            while seen != Some(want) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(250));
                let cs = platform::controls(w.id, 2000).unwrap_or_default();
                data["after"] = switches(&cs);
                seen = now_on(&cs, &was, which);
            }
            let outcome = if seen == Some(want) {
                Outcome::ok(format!("done: {done}"))
            } else {
                Outcome::unsure(format!("pressed Discord's \"{}\", but its panel hasn't shown the change yet; it may still have worked", was.control.name))
            };
            outcome.with(data)
        })
        .await
        .expect("blocking task panicked")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, left: i32, top: i32, toggled: Option<bool>) -> Control {
        Control { index: 0, name: name.into(), kind: "button".into(), rect: [left, top, left + 30, top + 30], enabled: true, toggled, focused: false, id: String::new() }
    }

    #[test]
    fn your_switch_is_the_one_that_toggles() {
        // As read from Discord in a call: the user panel's switch, and the call panel's plain button.
        let cs = vec![c("Mute", 456, 1197, Some(false)), c("Deafen", 519, 1197, Some(false)), c("Mute", 1152, 1197, None)];
        let m = mic(&cs).unwrap();
        assert_eq!((m.control.rect[0], m.on), (456, Some(false)));
        assert_eq!(ears(&cs).unwrap().on, Some(false));
    }

    #[test]
    fn renamed_while_on() {
        let cs = vec![c("Unmute", 456, 1197, None), c("Deafen", 519, 1197, Some(false))];
        assert_eq!(mic(&cs).unwrap().on, Some(true));
        // After pressing "Mute", the same place now holds a switch that's on.
        let was = Switch { control: c("Mute", 456, 1197, Some(false)), on: Some(false) };
        let after = vec![c("Unmute", 456, 1197, Some(true))];
        assert_eq!(now_on(&after, &was, mic), Some(true));
    }
}
