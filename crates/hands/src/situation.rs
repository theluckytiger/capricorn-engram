//! What she knows about the moment, written for her brain: the time, home, the open windows, what's playing.

use crate::tool::Ctx;

pub fn describe(ctx: &Ctx) -> String {
    let now = chrono::Local::now();
    let mut s = format!("Now: {} (local time).", now.format("%A %Y-%m-%d %H:%M"));
    match ctx.home.clone().or_else(|| capri_core::Settings::load().ok()?.home) {
        Some(h) => s.push_str(&format!("\nHome: {h}.")),
        None => s.push_str("\nHome: not known yet."),
    }
    #[cfg(windows)]
    {
        let wins: Vec<_> = crate::platform::list_windows().into_iter().filter(|w| w.pid != ctx.own_pid).take(10).collect();
        if wins.is_empty() {
            s.push_str("\nOpen windows: none.");
        } else {
            let list: Vec<String> = wins
                .iter()
                .map(|w| {
                    let title: String = w.title.chars().take(80).collect();
                    format!("\"{title}\" ({}{}{})", w.app, if w.foreground { ", in front" } else { "" }, if w.minimized { ", minimized" } else { "" })
                })
                .collect();
            s.push_str(&format!("\nOpen windows, front to back: {}.", list.join("; ")));
        }
        if let Ok(playing) = crate::platform::now_playing()
            && let Some(n) = playing.first()
        {
            s.push_str(&format!("\nMedia: {} \"{}\"{} in {}.", if n.playing { "playing" } else { "paused" }, n.title, if n.artist.is_empty() { String::new() } else { format!(" by {}", n.artist) }, n.app));
        }
    }
    s
}
