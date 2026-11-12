//! A strong brain's own eyes on a window: `look` shows it the window as a picture (the brain sees it, not her small
//! local eyes), and `click_at` clicks a spot on that picture, the way a person points at what they see. For the
//! thumbnail, the canvas, the game menu nothing names. Only brains that see pictures get these (her local one keeps
//! `see` and clicking by description), and a picture of your screen goes to a cloud brain only once you've said yes for
//! that job, or allowed it in her settings.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::eyes::{MAX_SIDE, encode};
use crate::pc::find_window;
use crate::platform::{self, WinAction, WinInfo};
use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

/// Where a window's last picture was taken: its screen corner, and how many screen pixels one picture pixel is.
type Corner = (i32, i32, f64);

/// Each window's last picture, by window.
static SHOTS: Mutex<Option<HashMap<isize, Corner>>> = Mutex::new(None);

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("blocking task panicked")
}

fn window(ctx: &Ctx, name: &str) -> Result<WinInfo, Outcome> {
    let wins: Vec<_> = platform::list_windows().into_iter().filter(|w| w.pid != ctx.own_pid).collect();
    find_window(name, &wins)
}

pub struct Look;

#[async_trait]
impl Tool for Look {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "look".into(),
            description: "See a window yourself, as a picture: what's on it and where. Use it before working inside a window, \
                          and again after a step to check what it did. Then click_at the spot you want in that picture. \
                          `window`: the app's name, words of its title, or \"front\"."
                .into(),
            parameters: json!({ "type": "object", "properties": { "window": { "type": "string" } }, "required": ["window"] }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Read
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let name: String = opt_arg(&args, "window").unwrap_or_else(|| "front".into());
        let w = match window(ctx, &name) {
            Ok(w) => w,
            Err(o) => return o,
        };
        let (id, raise) = (w.id, w.minimized && !ctx.dry_run);
        let shot = blocking(move || {
            if raise {
                let _ = platform::act(id, WinAction::Focus);
                std::thread::sleep(Duration::from_millis(350));
            }
            // Once it has settled (a page still loading looked empty).
            let mut shot = platform::capture(id)?;
            for _ in 0..8 {
                std::thread::sleep(Duration::from_millis(300));
                let Ok(next) = platform::capture(id) else { break };
                let moving = platform::changed(&shot, &next);
                shot = next;
                if moving < 0.03 {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(shot)
        })
        .await;
        let shot = match shot {
            Ok(s) => s,
            Err(e) => return Outcome::failed(format!("couldn't take a picture of {}: {e:#}", w.app)),
        };
        let pic = match encode(&shot.rgb, shot.width, shot.height, MAX_SIDE) {
            Ok(p) => p,
            Err(e) => return Outcome::failed(format!("{e:#}")),
        };
        let scale = shot.width as f64 / pic.width as f64;
        SHOTS.lock().unwrap().get_or_insert_with(HashMap::new).insert(w.id, (shot.left, shot.top, scale));
        // The picture rides in `image`: her brain takes it out and shows it to the model (never into her logs).
        Outcome::ok(format!("here's {} \"{}\" as a picture, {}x{}: click_at takes x and y in this picture's pixels", w.app, w.title, pic.width, pic.height))
            .with(json!({ "window": w.title, "image": pic.base64, "picture": [pic.width, pic.height], "rect": w.rect, "label": w.app }))
    }
}

pub struct ClickAt;

#[async_trait]
impl Tool for ClickAt {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "click_at".into(),
            description: "Click a spot on the picture look gave you of a window: x and y in that picture's pixels, `what` is \
                          what's there (\"the fifth video's thumbnail\"). `double`: a double click. Look again afterwards to \
                          see what it did."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "window": { "type": "string" },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "what": { "type": "string" },
                    "double": { "type": "boolean" }
                },
                "required": ["window", "x", "y", "what"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        if args.get("what").and_then(Value::as_str).is_some_and(crate::controls::risky) { Risk::Risky } else { Risk::Normal }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let (name, what): (String, String) = match (arg(&args, "window"), arg(&args, "what")) {
            (Ok(n), Ok(w)) => (n, w),
            (Err(o), _) | (_, Err(o)) => return o,
        };
        let (x, y): (f64, f64) = match (arg(&args, "x"), arg(&args, "y")) {
            (Ok(x), Ok(y)) => (x, y),
            (Err(o), _) | (_, Err(o)) => return o,
        };
        let double = opt_arg::<bool>(&args, "double").unwrap_or(false);
        let w = match window(ctx, &name) {
            Ok(w) => w,
            Err(o) => return o,
        };
        let Some((left, top, scale)) = SHOTS.lock().unwrap().get_or_insert_with(HashMap::new).get(&w.id).copied() else {
            return Outcome::failed(format!("look at {} first: click_at takes a spot on its picture", w.app));
        };
        let (sx, sy) = (left + (x * scale).round() as i32, top + (y * scale).round() as i32);
        let data = json!({ "window": w.title, "point": [sx, sy], "rect": [sx - 24, sy - 16, sx + 24, sy + 16], "label": what });
        if platform::caption_button_at(w.id, sx, sy) {
            return Outcome::failed(format!("that's {}'s own title-bar button (close, minimize or maximize), not something on its page: to close, minimize or maximize the window use the windows tool; a popup's ✕ is inside the page, so look again", w.app)).with(data);
        }
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would click \"{what}\" in {} at ({sx}, {sy})", w.app)).with(data);
        }
        let (id, before) = (w.id, w.title.clone());
        let done = blocking(move || {
            // The spot must still be this window's: something may have come over it since the picture.
            if platform::window_at_not_mine(sx, sy) != Some(id) {
                return Err("something else is over that spot now: look again".to_string());
            }
            let pic = platform::capture(id).ok();
            platform::click_point(sx, sy).map_err(|e| format!("{e:#}"))?;
            if double {
                std::thread::sleep(Duration::from_millis(60));
                platform::click_point(sx, sy).map_err(|e| format!("{e:#}"))?;
            }
            std::thread::sleep(Duration::from_millis(600));
            let moved = match (pic, platform::capture(id).ok()) {
                (Some(a), Some(b)) => platform::changed(&a, &b),
                _ => 1.0,
            };
            Ok((platform::list_windows().into_iter().find(|x| x.id == id).map(|x| x.title).unwrap_or_default(), moved))
        })
        .await;
        match done {
            Ok((after, _)) if after != before && !after.is_empty() => Outcome::ok(format!("clicked \"{what}\" in {}; the window is now \"{after}\"", w.app)).with(data),
            Ok((_, moved)) if moved < 0.004 => Outcome::unsure(format!("clicked \"{what}\" in {}, but nothing in the window changed: look again before saying it worked", w.app)).with(data),
            Ok(_) => Outcome::ok(format!("clicked \"{what}\" in {} (look again to see what it did)", w.app)).with(data),
            Err(e) => Outcome::failed(format!("couldn't click \"{what}\": {e}")).with(data),
        }
    }
}

pub struct DragAt;

#[async_trait]
impl Tool for DragAt {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "drag_at".into(),
            description: "Drag on the picture look gave you of a window, from one spot to another (a slider, a volume bar, a \
                          scroll bar, something to move): x and y in that picture's pixels, `what` is what's dragged."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "window": { "type": "string" },
                    "from_x": { "type": "number" },
                    "from_y": { "type": "number" },
                    "to_x": { "type": "number" },
                    "to_y": { "type": "number" },
                    "what": { "type": "string" }
                },
                "required": ["window", "from_x", "from_y", "to_x", "to_y", "what"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        if args.get("what").and_then(Value::as_str).is_some_and(crate::controls::risky) { Risk::Risky } else { Risk::Normal }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let (name, what): (String, String) = match (arg(&args, "window"), arg(&args, "what")) {
            (Ok(n), Ok(w)) => (n, w),
            (Err(o), _) | (_, Err(o)) => return o,
        };
        let n = |k: &str| args.get(k).and_then(Value::as_f64);
        let (Some(fx), Some(fy), Some(tx), Some(ty)) = (n("from_x"), n("from_y"), n("to_x"), n("to_y")) else {
            return Outcome::failed("drag_at needs from_x, from_y, to_x and to_y");
        };
        let w = match window(ctx, &name) {
            Ok(w) => w,
            Err(o) => return o,
        };
        let Some((left, top, scale)) = SHOTS.lock().unwrap().get_or_insert_with(HashMap::new).get(&w.id).copied() else {
            return Outcome::failed(format!("look at {} first: drag_at takes spots on its picture", w.app));
        };
        let at = |x: f64, y: f64| (left + (x * scale).round() as i32, top + (y * scale).round() as i32);
        let (from, to) = (at(fx, fy), at(tx, ty));
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would drag \"{what}\" in {} from {from:?} to {to:?}", w.app));
        }
        let id = w.id;
        let done = blocking(move || {
            if platform::window_at_not_mine(from.0, from.1) != Some(id) {
                return Err("something else is over that spot now: look again".to_string());
            }
            platform::drag_points(from, to).map_err(|e| format!("{e:#}"))
        })
        .await;
        match done {
            Ok(()) => Outcome::ok(format!("dragged \"{what}\" in {} (look again to see what it did)", w.app)),
            Err(e) => Outcome::failed(format!("couldn't drag \"{what}\": {e}")),
        }
    }
}

pub struct Scroll;

#[async_trait]
impl Tool for Scroll {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "scroll".into(),
            description: "Scroll a window up or down by pages, or to its top or bottom, with its keys (your pointer stays \
                          where it is). For more of a page, a feed, comments, a long list. `window`: the app's name, words \
                          of its title, or \"front\"."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "window": { "type": "string" },
                    "direction": { "type": "string", "enum": ["down", "up", "top", "bottom"] },
                    "pages": { "type": "integer", "minimum": 1, "maximum": 10 }
                },
                "required": ["direction"]
            }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Normal
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let name: String = opt_arg(&args, "window").unwrap_or_else(|| "front".into());
        let direction: String = opt_arg(&args, "direction").unwrap_or_else(|| "down".into());
        let pages = opt_arg::<u32>(&args, "pages").unwrap_or(1).clamp(1, 10);
        let (key, times) = match direction.as_str() {
            "up" => ("pageup", pages),
            "top" => ("ctrl+home", 1),
            "bottom" => ("ctrl+end", 1),
            _ => ("pagedown", pages),
        };
        let w = match window(ctx, &name) {
            Ok(w) => w,
            Err(o) => return o,
        };
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would press {key} {times}x in {}", w.app));
        }
        let id = w.id;
        let moved = blocking(move || {
            let _ = platform::act(id, WinAction::Focus);
            std::thread::sleep(Duration::from_millis(150));
            let before = platform::capture(id).ok();
            for _ in 0..times {
                platform::press(key)?;
                std::thread::sleep(Duration::from_millis(120));
            }
            std::thread::sleep(Duration::from_millis(350));
            Ok::<f32, anyhow::Error>(match (before, platform::capture(id).ok()) {
                (Some(a), Some(b)) => platform::changed(&a, &b),
                _ => 1.0,
            })
        })
        .await;
        match moved {
            Ok(m) if m < 0.004 => Outcome::unsure(format!("pressed {key} in {}, but nothing moved: it may be at the end already, or the page needs a click inside first", w.app)),
            Ok(_) => Outcome::ok(format!("scrolled {direction} in {}", w.app)),
            Err(e) => Outcome::failed(format!("{e:#}")),
        }
    }
}
