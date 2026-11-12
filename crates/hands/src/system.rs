//! `system`: facts about this PC (how busy it is, memory, disks, battery, uptime, its hardware, what uses the most
//! memory), and locking it.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg};

/// Bytes as people say them: "12.4 GB".
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1_073_741_824.0;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1_048_576.0) }
}

/// A duration as people say it: "3 days 4 h", "2 h 10 min", "25 min".
pub fn span(d: Duration) -> String {
    let m = d.as_secs() / 60;
    let (days, h, min) = (m / 1440, (m / 60) % 24, m % 60);
    if days > 0 {
        format!("{days} day{} {h} h", if days == 1 { "" } else { "s" })
    } else if h > 0 {
        format!("{h} h {min} min")
    } else {
        format!("{min} min")
    }
}

pub struct System;

#[async_trait]
impl Tool for System {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "system".into(),
            description: "This PC's state: `overview` (processor load, memory, disks, battery, uptime, hardware, Windows), \
                          or one of `cpu`, `memory`, `disks`, `battery`, `uptime`, `hardware`, `processes` (what uses \
                          the most memory). `lock` locks the PC (the sign-in screen; nothing closes). `dark` / `light`: Windows in dark or light \
                          mode (apps and taskbar)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "what": { "type": "string", "enum": ["overview", "cpu", "memory", "disks", "battery", "uptime", "hardware", "processes", "lock", "dark", "light"] } },
                "required": ["what"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        if matches!(args.get("what").and_then(Value::as_str), Some("lock" | "dark" | "light")) { Risk::Normal } else { Risk::Read }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let what: String = match arg(&args, "what") {
            Ok(w) => w,
            Err(o) => return o,
        };
        let dry = ctx.dry_run;
        tokio::task::spawn_blocking(move || facts(&what, dry)).await.unwrap_or_else(|e| Outcome::failed(format!("{e}")))
    }
}

#[cfg(windows)]
fn facts(what: &str, dry: bool) -> Outcome {
    use crate::platform::system as sys;
    let mut lines = Vec::new();
    let all = what == "overview";
    if all || what == "cpu" {
        match sys::cpu_load(Duration::from_millis(500)) {
            Some(p) => lines.push(format!("processor: {p:.0}% busy")),
            None => lines.push("processor load: unknown".into()),
        }
    }
    if (all || what == "memory")
        && let Some((used, total)) = sys::memory()
    {
        lines.push(format!("memory: {} of {} in use ({:.0}%)", size(used), size(total), used as f64 * 100.0 / total.max(1) as f64));
    }
    if all || what == "disks" {
        for (drive, free, total) in sys::disks() {
            lines.push(format!("{drive} {} free of {}", size(free), size(total)));
        }
    }
    if all || what == "battery" {
        lines.push(match sys::battery() {
            Some((p, charging)) => format!("battery: {p}%{}", if charging { ", charging" } else { "" }),
            None => "battery: none (a desktop)".into(),
        });
    }
    if all || what == "uptime" {
        lines.push(format!("on for {}", span(sys::uptime())));
    }
    if all || what == "hardware" {
        let (cpu, gpu, os) = sys::names();
        lines.extend([cpu.map(|c| format!("processor: {c}")), gpu.map(|g| format!("graphics: {g}")), os.map(|o| format!("system: {o}"))].into_iter().flatten());
    }
    if what == "processes" {
        let names: std::collections::HashMap<u32, String> = crate::platform::list_windows().into_iter().map(|w| (w.pid, w.app)).collect();
        for (pid, bytes) in sys::biggest(8) {
            let name = names.get(&pid).cloned().or_else(|| crate::platform::exe_path(pid).and_then(|p| std::path::Path::new(&p).file_stem().map(|s| s.to_string_lossy().into_owned()))).unwrap_or_else(|| format!("process {pid}"));
            lines.push(format!("{name}: {}", size(bytes)));
        }
    }
    if what == "lock" {
        if dry {
            return Outcome::ok("(dry run) would lock the PC");
        }
        return match sys::lock() {
            Ok(()) => Outcome::ok("locked the PC"),
            Err(e) => Outcome::failed(format!("couldn't lock it: {e:#}")),
        };
    }
    if what == "dark" || what == "light" {
        let dark = what == "dark";
        if sys::is_dark() == Some(dark) {
            return Outcome::ok(format!("Windows is already in {what} mode"));
        }
        if dry {
            return Outcome::ok(format!("(dry run) would switch Windows to {what} mode"));
        }
        return match sys::set_dark(dark) {
            Ok(()) if sys::is_dark() == Some(dark) => Outcome::ok(format!("switched Windows to {what} mode")),
            Ok(()) => Outcome::unsure(format!("asked Windows for {what} mode, but it still reads the other")),
            Err(e) => Outcome::failed(format!("couldn't switch to {what} mode: {e:#}")),
        };
    }
    if lines.is_empty() {
        return Outcome::failed(format!("unknown: \"{what}\""));
    }
    Outcome::ok(lines.join("; "))
}

#[cfg(not(windows))]
fn facts(_: &str, _: bool) -> Outcome {
    Outcome::failed("not on this OS yet")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_people_say_them() {
        assert_eq!(size(13_314_398_618), "12.4 GB");
        assert_eq!(size(524_288_000), "500 MB");
        assert_eq!(span(Duration::from_secs(3 * 86400 + 4 * 3600 + 120)), "3 days 4 h");
        assert_eq!(span(Duration::from_secs(2 * 3600 + 600)), "2 h 10 min");
        assert_eq!(span(Duration::from_secs(25 * 60)), "25 min");
    }
}
