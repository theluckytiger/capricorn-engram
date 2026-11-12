//! Keeping an eye out: "tell me when the download finishes", "let me know when my health is low". She looks at that
//! window every so often with her own eyes (on this PC), asks them whether it has happened yet, and when it has, tells
//! you through her reminders (they get your attention the same way). A watch ends when it happens, when its window
//! closes, when its time is up, or when you say stop.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::eyes::{MAX_SIDE, encode};
use crate::pc::find_window;
use crate::platform;
use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

struct Watching {
    id: u64,
    what: String,
    stop: Arc<AtomicBool>,
}

static WATCHES: Mutex<Vec<Watching>> = Mutex::new(Vec::new());

pub struct Watch;

#[async_trait]
impl Tool for Watch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "watch".into(),
            description: "Keep an eye on a window and tell the user when something happens on it (\"tell me when the \
                          download finishes\", \"let me know when my health is low\"). action start: `window`, `until` (what \
                          to wait for, as it would look on screen: \"the download shows 100% or Done\"), `say` (what to tell \
                          them then), `every` seconds (default 20), `minutes` to keep at it (default 60). action list, or \
                          action stop (all, or `what` words of one)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["start", "list", "stop"] },
                    "window": { "type": "string" },
                    "until": { "type": "string" },
                    "say": { "type": "string" },
                    "every": { "type": "integer", "minimum": 5, "maximum": 600 },
                    "minutes": { "type": "integer", "minimum": 1, "maximum": 720 },
                    "what": { "type": "string" }
                },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Read
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = opt_arg(&args, "action").unwrap_or_else(|| "start".into());
        match action.as_str() {
            "list" => {
                let w = WATCHES.lock().unwrap();
                if w.is_empty() { Outcome::ok("not watching anything") } else { Outcome::ok(format!("watching: {}", w.iter().map(|x| x.what.clone()).collect::<Vec<_>>().join("; "))) }
            }
            "stop" => {
                let words: String = opt_arg::<String>(&args, "what").unwrap_or_default().to_lowercase();
                let mut w = WATCHES.lock().unwrap();
                let before = w.len();
                w.retain(|x| {
                    let hit = words.is_empty() || x.what.to_lowercase().contains(&words);
                    if hit {
                        x.stop.store(true, Ordering::Relaxed);
                    }
                    !hit
                });
                Outcome::ok(format!("stopped {} watch{}", before - w.len(), if before - w.len() == 1 { "" } else { "es" }))
            }
            _ => start(args, ctx),
        }
    }
}

fn start(args: Value, ctx: &Ctx) -> Outcome {
    let (name, until): (String, String) = match (arg(&args, "window"), arg(&args, "until")) {
        (Ok(n), Ok(u)) => (n, u),
        (Err(o), _) | (_, Err(o)) => return o,
    };
    if transient_until(&until) {
        return Outcome::failed(
            "won't watch for a transient splash (\"is loading\", \"signing in\", \"please wait\"). Watch for a real \
             change (Done, 100%, ready, finished), or open the app and check once with see.",
        );
    }
    let say: String = opt_arg(&args, "say").unwrap_or_else(|| format!("{until}."));
    let every = Duration::from_secs(opt_arg::<u64>(&args, "every").unwrap_or(20).clamp(5, 600));
    let minutes = opt_arg::<u64>(&args, "minutes").unwrap_or(60).clamp(1, 720);
    let Some(eyes) = ctx.eyes.clone() else { return Outcome::failed("her eyes are off (no vision model on this PC), so she can't watch a window") };
    let wins: Vec<_> = platform::list_windows().into_iter().filter(|w| w.pid != ctx.own_pid).collect();
    let w = match find_window(&name, &wins) {
        Ok(w) => w,
        Err(o) if o.summary.starts_with("no open window matches") => {
            return Outcome::failed(format!(
                "\"{name}\" isn't open, so she can't watch it. Open it first (open name \"{name}\"); if it isn't \
                 installed, say so — don't explore other windows looking for it."
            ));
        }
        Err(o) => return o,
    };
    if ctx.dry_run {
        return Outcome::ok(format!("(dry run) would watch {} for: {until}", w.app));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let id = {
        let mut list = WATCHES.lock().unwrap();
        let id = list.iter().map(|x| x.id).max().unwrap_or(0) + 1;
        list.push(Watching { id, what: format!("{} for \"{until}\"", w.app), stop: stop.clone() });
        id
    };
    let (window, app) = (w.id, w.app.clone());
    let told = format!("watching {} every {} s for: {until} (up to {minutes} min); she'll tell them when it happens", w.app, every.as_secs());
    let question = format!("Is this true on it right now: {until}? Answer yes or no first, then say in a few words what you see.");
    tokio::spawn(async move {
        let end = Instant::now() + Duration::from_secs(minutes * 60);
        let done = |id: u64| WATCHES.lock().unwrap().retain(|x| x.id != id);
        while Instant::now() < end && !stop.load(Ordering::Relaxed) {
            tokio::time::sleep(every).await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let shot = tokio::task::spawn_blocking(move || platform::capture(window)).await;
            let Ok(Ok(shot)) = shot else {
                // Its window is gone (closed, or minimized for good): nothing more to watch.
                if !platform::list_windows().iter().any(|x| x.id == window) {
                    let _ = capri_core::reminders::add(chrono::Local::now(), &format!("Heads up: {app} closed, so I stopped watching it for \"{until}\"."));
                    done(id);
                    return;
                }
                continue;
            };
            let Ok(pic) = encode(&shot.rgb, shot.width, shot.height, MAX_SIDE) else { continue };
            let Ok(answer) = eyes.0.look(&pic, &crate::eyes::see_prompt(&question)).await else { continue };
            if answer.trim_start().to_lowercase().starts_with("yes") {
                let _ = capri_core::reminders::add(chrono::Local::now(), &format!("Heads up: {}", say.trim()));
                done(id);
                return;
            }
        }
        if !stop.load(Ordering::Relaxed) {
            let _ = capri_core::reminders::add(chrono::Local::now(), &format!("Heads up: I watched {app} for {minutes} minutes and \"{until}\" didn't happen, so I stopped."));
        }
        done(id);
    });
    Outcome::ok(told)
}

/// Watching along with you: "watch this with me".
static ALONG: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

pub struct WatchAlong;

#[async_trait]
impl Tool for WatchAlong {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "watch_along".into(),
            description: "Watch a video, stream or game with the user (\"watch this with me\"): she looks at it now and then \
                          and reacts out loud, like a friend on the couch. action start (`window`, default the one in front; \
                          `minutes`, default 30) or stop."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "action": { "type": "string", "enum": ["start", "stop"] }, "window": { "type": "string" }, "minutes": { "type": "integer", "minimum": 1, "maximum": 240 } },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Read
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = opt_arg(&args, "action").unwrap_or_else(|| "start".into());
        if let Some(old) = ALONG.lock().unwrap().take() {
            old.store(true, Ordering::Relaxed);
            if action == "stop" {
                return Outcome::ok("stopped watching along");
            }
        } else if action == "stop" {
            return Outcome::ok("wasn't watching anything");
        }
        let (Some(eyes), Some(aside)) = (ctx.eyes.clone(), ctx.aside.clone()) else { return Outcome::failed("her eyes are off (no vision model on this PC), so she can't watch along") };
        let name: String = opt_arg(&args, "window").unwrap_or_else(|| "front".into());
        let minutes = opt_arg::<u64>(&args, "minutes").unwrap_or(30).clamp(1, 240);
        let wins: Vec<_> = platform::list_windows().into_iter().filter(|w| w.pid != ctx.own_pid).collect();
        let w = match find_window(&name, &wins) {
            Ok(w) => w,
            Err(o) => return o,
        };
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would watch {} along with them", w.app));
        }
        let stop = Arc::new(AtomicBool::new(false));
        *ALONG.lock().unwrap() = Some(stop.clone());
        let window = w.id;
        tokio::spawn(async move {
            let end = Instant::now() + Duration::from_secs(minutes * 60);
            let mut last = String::new();
            // The first look soon, then every 40 to 70 s (not like clockwork).
            let mut wait = Duration::from_secs(8);
            while Instant::now() < end && !stop.load(Ordering::Relaxed) {
                tokio::time::sleep(wait).await;
                wait = Duration::from_secs(40 + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64 % 30).unwrap_or(0));
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let shot = tokio::task::spawn_blocking(move || platform::capture(window)).await;
                let Ok(Ok(shot)) = shot else {
                    if !platform::list_windows().iter().any(|x| x.id == window) {
                        break;
                    }
                    continue;
                };
                let Ok(pic) = encode(&shot.rgb, shot.width, shot.height, MAX_SIDE) else { continue };
                let q = "What's happening in this right now (a video, a stream or a game)? One or two sentences, with any words on screen worth reacting to.";
                let Ok(seen) = eyes.0.look(&pic, &crate::eyes::see_prompt(q)).await else { continue };
                let seen = seen.trim().to_string();
                // Nothing new on screen (paused, a menu): nothing to say.
                if seen.is_empty() || crate::matching::norm(&seen) == crate::matching::norm(&last) {
                    continue;
                }
                last = seen.clone();
                (aside.0)(seen);
            }
            let mut a = ALONG.lock().unwrap();
            if a.as_ref().is_some_and(|s| Arc::ptr_eq(s, &stop)) {
                *a = None;
            }
        });
        Outcome::ok(format!("watching {} along with them for up to {minutes} minutes; she'll react now and then (\"stop watching\" ends it)", w.app))
    }
}

/// A splash / loading line, not a real event worth a background watch.
fn transient_until(until: &str) -> bool {
    let u = until.to_lowercase();
    const BAD: &[&str] = &["is loading", "still loading", "loading…", "loading...", "signing in", "signing into", "please wait", "starting up", "splash"];
    let splash = BAD.iter().any(|p| u.contains(p)) || u.trim() == "loading";
    let done = ["finish", "finished", "done", "complete", "100%", "ready", "downloaded", "starts", "started", "online"].iter().any(|g| u.contains(g));
    splash && !done
}

#[cfg(test)]
mod tests {
    #[test]
    fn splash_watches_are_refused() {
        assert!(super::transient_until("Steam is loading"));
        assert!(super::transient_until("signing in"));
        assert!(!super::transient_until("the download finishes"));
        assert!(!super::transient_until("Steam finished loading"));
    }
}
