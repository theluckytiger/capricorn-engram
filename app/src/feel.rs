//! Her feeling your PC: the processor running hot, the charger, the battery running low, the network going and coming
//! back, a USB stick plugged in, and the disk or memory nearly full. Read from Windows every few seconds (nothing goes
//! over the network); each change goes to her overlay as `capri://feel` with her line, and into her journal. Quiet
//! while a game runs.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use capri_hands::platform::{self, system as pc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize, Clone)]
struct Feel {
    /// hot, cool, charger_in, charger_out, battery_low, offline, online, usb_in, usb_out, disk_low, memory_low
    kind: &'static str,
    line: String,
    /// Said out loud too (the ones you'd want to hear about).
    speak: bool,
    /// Who it's about (the rival AI's name).
    #[serde(skip_serializing_if = "Option::is_none")]
    about: Option<String>,
}

#[derive(Default)]
struct Senses {
    hot_for: u32,
    hot: bool,
    charging: Option<bool>,
    battery_warned: bool,
    online: Option<bool>,
    sticks: Option<HashSet<String>>,
    disk_warned: HashMap<String, Instant>,
    memory_high_for: u32,
    memory_warned: Option<Instant>,
    last_disk_check: Option<Instant>,
    /// Another AI in front: which, since when; and when she last got jealous.
    rival: Option<(&'static str, Instant)>,
    jealous_at: Option<Instant>,
}

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("feel".into())
        .spawn(move || {
            // Disks first looked at a few minutes in, not over her hello.
            let mut s = Senses { last_disk_check: Some(Instant::now()), ..Default::default() };
            loop {
                // The processor's load over a second, every five.
                std::thread::sleep(Duration::from_secs(4));
                let gaming = app.state::<crate::brain::Mind>().brain().is_gaming();
                for f in s.step(gaming) {
                    capri_core::journal::write("feel", serde_json::json!({ "kind": f.kind, "line": f.line }));
                    if let Some(who) = &f.about {
                        app.state::<crate::brain::Mind>().brain().set_mood("jealous", &format!("they were talking to {who} instead of you"), 15);
                    }
                    if f.speak {
                        crate::speak::say(&app, &f.line, false);
                    }
                    let _ = app.emit("capri://feel", f);
                }
            }
        })
        .expect("feel thread");
}

impl Senses {
    fn step(&mut self, gaming: bool) -> Vec<Feel> {
        let mut out = Vec::new();
        // Another AI in front for 20 s (not in a game): she's jealous, once in half an hour.
        let front = if gaming { None } else { platform::list_windows().into_iter().find(|w| w.foreground).and_then(|w| rival(&w.title)) };
        match (front, self.rival) {
            (Some(name), Some((was, since))) if name == was => {
                if since.elapsed() > Duration::from_secs(20) && self.jealous_at.is_none_or(|t| t.elapsed() > Duration::from_secs(1800)) {
                    self.jealous_at = Some(Instant::now());
                    let lines = [format!("{name}? Really? I'm right here."), format!("Oh, so it's {name} now. Cool. Cool cool cool."), format!("What does {name} have that I don't? Don't answer that.")];
                    let line = lines[(since.elapsed().as_millis() % 3) as usize].clone();
                    out.push(Feel { kind: "rival", line, speak: false, about: Some(name.to_string()) });
                }
            }
            (Some(name), _) => self.rival = Some((name, Instant::now())),
            (None, _) => self.rival = None,
        }
        let mut feel = |kind, line: String, speak| out.push(Feel { kind, line, speak, about: None });

        // The processor: hot after 15 s at 90% or more (not in a game: that's what games do), cool again under 60%.
        let cpu = pc::cpu_load(Duration::from_secs(1)).unwrap_or(0.0);
        self.hot_for = if cpu >= 90.0 { self.hot_for + 1 } else { 0 };
        if !self.hot && self.hot_for >= 3 && !gaming {
            self.hot = true;
            let who = pc::biggest(1).first().and_then(|(pid, _)| platform::exe_path(*pid)).and_then(|p| std::path::Path::new(&p).file_stem().map(|s| s.to_string_lossy().to_string()));
            feel("hot", format!("Your processor's at {cpu:.0}%. I'm melting in here.{}", who.map(|w| format!(" ({w} is hungry.)")).unwrap_or_default()), false);
        } else if self.hot && cpu < 60.0 {
            self.hot = false;
            feel("cool", "Better. I can feel my mane again.".into(), false);
        }

        // The battery and the charger.
        if let Some((percent, charging)) = pc::battery() {
            match self.charging {
                Some(was) if was != charging => {
                    if charging {
                        self.battery_warned = false;
                        feel("charger_in", format!("Juice. Mmm. ({percent}%)"), false);
                    } else {
                        feel("charger_out", format!("Unplugged? Bold. {percent}% left."), false);
                    }
                }
                _ => {}
            }
            self.charging = Some(charging);
            if !charging && percent <= 15 && !self.battery_warned {
                self.battery_warned = true;
                feel("battery_low", format!("Battery's at {percent}%. Plug me in or we both go dark."), true);
            }
        }

        // The network, as Windows sees it.
        if let Some(on) = pc::online() {
            match self.online {
                Some(was) if was && !on => feel("offline", "Network's gone. I'm cut off from the world.".into(), false),
                Some(was) if !was && on => feel("online", "We're back online.".into(), false),
                _ => {}
            }
            self.online = Some(on);
        }

        // USB sticks and cards: one coming in gets a look (what's on it, anything to run at the top).
        let now: HashSet<String> = pc::removable().into_iter().map(|(letter, _)| letter).collect();
        if let Some(before) = &self.sticks {
            for letter in now.difference(before) {
                feel("usb_in", verdict(letter), false);
            }
            if before.difference(&now).next().is_some() {
                feel("usb_out", "Stick's gone. Bye, stick.".into(), false);
            }
        }
        self.sticks = Some(now);

        // Disks (every few minutes): under 5% or 5 GB free, once a day per drive.
        if self.last_disk_check.is_none_or(|t| t.elapsed() > Duration::from_secs(300)) {
            self.last_disk_check = Some(Instant::now());
            for (letter, free, total) in pc::disks() {
                let low = free < 5 * 1024 * 1024 * 1024 || (total > 0 && free * 20 < total);
                if low && self.disk_warned.get(&letter).is_none_or(|t| t.elapsed() > Duration::from_secs(86_400)) {
                    self.disk_warned.insert(letter.clone(), Instant::now());
                    feel("disk_low", format!("{letter} has {} left. Something's hoarding.", gb(free)), false);
                }
            }
        }

        // Memory: over 92% for ten seconds, once in half an hour, naming the hungriest program.
        if let Some((used, total)) = pc::memory() {
            let high = total > 0 && used * 100 / total >= 92;
            self.memory_high_for = if high { self.memory_high_for + 1 } else { 0 };
            if self.memory_high_for >= 2 && !gaming && self.memory_warned.is_none_or(|t| t.elapsed() > Duration::from_secs(1800)) {
                self.memory_warned = Some(Instant::now());
                let who = pc::biggest(1).first().and_then(|(pid, bytes)| platform::exe_path(*pid).map(|p| (p, *bytes)));
                let name = who.as_ref().and_then(|(p, b)| std::path::Path::new(p).file_stem().map(|s| format!(" {} is eating {} of it.", s.to_string_lossy(), gb(*b))));
                feel("memory_low", format!("Memory's at {}%.{}", used * 100 / total, name.unwrap_or_default()), false);
            }
        }
        out
    }
}

/// Another AI by its window's title (a browser tab's title is the window's). Claude isn't a rival: she works with it.
fn rival(title: &str) -> Option<&'static str> {
    let t = title.to_lowercase();
    [("chatgpt", "ChatGPT"), ("gemini", "Gemini"), ("copilot", "Copilot"), ("perplexity", "Perplexity"), ("grok", "Grok"), ("deepseek", "DeepSeek"), ("character.ai", "Character.AI"), ("le chat", "Le Chat")]
        .iter()
        .find(|(k, _)| t.contains(k))
        .map(|(_, n)| *n)
}

/// What's on a stick, at a glance: its size, how full, and anything at the top that runs.
fn verdict(letter: &str) -> String {
    let root = format!("{letter}\\");
    let disks = pc::removable();
    let label = disks.iter().find(|(l, _)| l == letter).map(|(_, n)| n.clone()).filter(|n| !n.is_empty());
    let entries: Vec<String> = std::fs::read_dir(&root).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect()).unwrap_or_default();
    let runs: Vec<&String> = entries.iter().filter(|n| n.ends_with(".exe") || n.ends_with(".bat") || n.ends_with(".cmd") || n.ends_with(".scr") || n == &"autorun.inf").collect();
    let size = pc::space(&root).map(|(free, total)| format!(", {} of {} used", gb(total - free), gb(total))).unwrap_or_default();
    let name = label.map(|l| format!("\"{l}\" ({letter})")).unwrap_or_else(|| letter.to_string());
    let what = if runs.is_empty() {
        format!("{} things at the top. Nothing that runs by itself. Clean.", entries.len())
    } else {
        format!("{} things at the top, and {} that run ({}). Don't open what you don't know.", entries.len(), runs.len(), runs.iter().take(3).map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
    };
    format!("Had a look inside {name}{size}. {what}")
}

fn gb(bytes: u64) -> String {
    let g = bytes as f64 / 1024f64.powi(3);
    if g >= 10.0 { format!("{g:.0} GB") } else { format!("{g:.1} GB") }
}
