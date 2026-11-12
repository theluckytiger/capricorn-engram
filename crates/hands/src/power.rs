//! Shutting down, restarting or sleeping this PC, now or in a while ("shut down in an hour", "sleep in 30 minutes"),
//! and calling it off. Always asked first. Shutting down and restarting go through Windows' own timer (it shows its
//! notice, and it holds even if she's closed); sleep is hers. She says it a minute before, so there's time to stop it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

/// A sleep she's waiting to do, and its stop flag.
static SLEEP: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

/// Her heads-up a minute before, by kind (to take back on a cancel).
fn heads_up(action: &str) -> String {
    let what = match action {
        "restart" => "the PC restarts",
        "sleep" => "the PC goes to sleep",
        _ => "the PC shuts down",
    };
    format!("Heads up: {what} in a minute. Say \"cancel the {action}\" to stop it.")
}

pub struct Power;

#[async_trait]
impl Tool for Power {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "power".into(),
            description: "Shut down, restart or put this PC to sleep, now or later (\"shut down in an hour\", \"sleep in \
                          30 minutes\"), or call it off. `action`: shutdown, restart, sleep or cancel; `in_minutes` (none: \
                          in a minute, a last chance to stop it). The user is asked first; she warns a minute before."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "action": { "type": "string", "enum": ["shutdown", "restart", "sleep", "cancel"] }, "in_minutes": { "type": "number", "minimum": 0 } },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        if args.get("action").and_then(Value::as_str) == Some("cancel") { Risk::Normal } else { Risk::Risky }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        let minutes = opt_arg::<f64>(&args, "in_minutes").unwrap_or(0.0).clamp(0.0, 60.0 * 24.0 * 7.0);
        // Never this instant: a minute at the least.
        let secs = ((minutes * 60.0).round() as u64).max(60);
        let when = if secs <= 60 { "in a minute".to_string() } else { format!("at {}", (chrono::Local::now() + chrono::Duration::seconds(secs as i64)).format("%H:%M")) };
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would {}", if action == "cancel" { "call off a shutdown, restart or sleep".to_string() } else { format!("{action} the PC {when}") }));
        }
        match action.as_str() {
            "shutdown" | "restart" => {
                let flag = if action == "shutdown" { "/s" } else { "/r" };
                let note = format!("Capri: {} {when}. Ask her to cancel it, or run shutdown /a.", if action == "shutdown" { "shutting down" } else { "restarting" });
                match std::process::Command::new("shutdown").args([flag, "/t", &secs.to_string(), "/c", &note]).output() {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => return Outcome::failed(format!("Windows said no: {}", String::from_utf8_lossy(&o.stderr).trim())),
                    Err(e) => return Outcome::failed(format!("couldn't ask Windows: {e}")),
                }
                warn(&action, secs);
                Outcome::ok(format!("the PC will {action} {when} (anything unsaved then is lost); \"cancel the {action}\" stops it"))
            }
            "sleep" => {
                let stop = Arc::new(AtomicBool::new(false));
                if let Some(old) = SLEEP.lock().unwrap().replace(stop.clone()) {
                    old.store(true, Ordering::Relaxed);
                }
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    if !stop.load(Ordering::Relaxed) {
                        let _ = crate::platform::system::sleep();
                    }
                });
                warn("sleep", secs);
                Outcome::ok(format!("the PC goes to sleep {when}; \"cancel the sleep\" stops it"))
            }
            "cancel" => {
                let slept = SLEEP.lock().unwrap().take().map(|s| s.store(true, Ordering::Relaxed)).is_some();
                // Exit code 1116: there was no shutdown to call off.
                let shut = std::process::Command::new("shutdown").arg("/a").output().is_ok_and(|o| o.status.success());
                let _ = capri_core::reminders::remove(|r| !r.text.starts_with("Heads up: the PC "));
                match (shut, slept) {
                    (false, false) => Outcome::ok("nothing was set to shut down, restart or sleep"),
                    (true, false) => Outcome::ok("called off the shutdown (or restart)"),
                    (false, true) => Outcome::ok("called off the sleep"),
                    (true, true) => Outcome::ok("called off the shutdown and the sleep"),
                }
            }
            other => Outcome::failed(format!("unknown action \"{other}\"")),
        }
    }
}

/// Her heads-up, said a minute before (when it's further off than that).
fn warn(action: &str, secs: u64) {
    let _ = capri_core::reminders::remove(|r| !r.text.starts_with("Heads up: the PC "));
    if secs > 120 {
        let _ = capri_core::reminders::add(chrono::Local::now() + chrono::Duration::seconds(secs as i64 - 60), &heads_up(action));
    }
}
