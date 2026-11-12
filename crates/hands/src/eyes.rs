//! Her eyes: a window's picture, looked at by a vision model (the `eyes` model, local unless you allow the cloud).
//! `see` answers questions about what's on a window; `click` falls back to looking when a window has no controls to
//! read (games, launchers, canvas apps, video players): the model says where the thing is, she checks the spot is
//! still that window's, and clicks it.
//!
//! The model is asked for points in thousandths of the picture (0–1000 across and down), the scale Qwen-VL models are
//! trained on; the picture is scaled down so its longer side is at most `MAX_SIDE` pixels.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::Value;

/// The longest side of a picture sent to the model: enough to read a launcher's text, few enough tokens to be quick.
pub const MAX_SIDE: u32 = 1280;

/// A picture for the model: PNG, base64.
#[derive(Debug, Clone)]
pub struct Picture {
    pub base64: String,
    pub width: u32,
    pub height: u32,
}

/// A vision model, as the hands see it (the brain provides it).
#[async_trait]
pub trait Look: Send + Sync {
    /// The model's answer to `prompt` about `picture`.
    async fn look(&self, picture: &Picture, prompt: &str) -> Result<String>;
}

/// Her eyes, if she has them (a vision model she's allowed to show the screen to).
#[derive(Clone)]
pub struct Eyes(pub Arc<dyn Look>);

impl std::fmt::Debug for Eyes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Eyes")
    }
}

/// RGB pixels (top row first) to a picture for the model, scaled down (by averaging) to fit `max_side`.
pub fn encode(rgb: &[u8], width: u32, height: u32, max_side: u32) -> Result<Picture> {
    let (png, w, h) = png_file(rgb, width, height, max_side)?;
    Ok(Picture { base64: base64::engine::general_purpose::STANDARD.encode(&png), width: w, height: h })
}

/// RGB pixels (top row first) to a PNG file's bytes, scaled down (by averaging) to fit `max_side`, and its size.
pub fn png_file(rgb: &[u8], width: u32, height: u32, max_side: u32) -> Result<(Vec<u8>, u32, u32)> {
    let scale = (max_side as f64 / width.max(height) as f64).min(1.0);
    let (w, h) = (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1));
    let pixels = if (w, h) == (width, height) { rgb.to_vec() } else { shrink(rgb, width, height, w, h) };
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        enc.write_header()?.write_image_data(&pixels)?;
    }
    Ok((png, w, h))
}

/// Area averaging: each new pixel is the mean of the old ones it covers.
fn shrink(rgb: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((nw * nh * 3) as usize);
    for y in 0..nh {
        let (y0, y1) = ((y * h / nh) as usize, (((y + 1) * h / nh).max(y * h / nh + 1)) as usize);
        for x in 0..nw {
            let (x0, x1) = ((x * w / nw) as usize, (((x + 1) * w / nw).max(x * w / nw + 1)) as usize);
            let mut sum = [0u32; 3];
            for row in y0..y1.min(h as usize) {
                let base = row * w as usize * 3;
                for col in x0..x1.min(w as usize) {
                    let i = base + col * 3;
                    sum[0] += rgb[i] as u32;
                    sum[1] += rgb[i + 1] as u32;
                    sum[2] += rgb[i + 2] as u32;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as u32;
            out.extend(sum.map(|s| (s / n) as u8));
        }
    }
    out
}

/// Where the model saw something: across and down, as fractions of the picture (0..1), and what's written on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
    pub label: String,
}

/// The prompt for finding one thing on the picture.
pub fn locate_prompt(what: &str) -> String {
    format!(
        "This is a screenshot of one window. Find this on it: {what}\n\
         Reply with JSON only, nothing else. If it's there: {{\"found\": true, \"x\": X, \"y\": Y, \"label\": L}}, where X \
         and Y are the centre of it in thousandths of the picture (0 is the left or top edge, 1000 the right or bottom \
         edge) and L is the words written on it (\"\" if it's only an icon). If it isn't there: {{\"found\": false, \
         \"seen\": S}}, where S says in a few words what is there instead."
    )
}

/// The model's reply to `locate_prompt`: `Ok(spot)`, or `Err(what it saw instead)`. Also takes the shapes Qwen-VL
/// models answer in by habit (`point_2d`, `bbox_2d`).
pub fn parse_spot(reply: &str) -> Result<Spot, String> {
    let json = reply.find('{').and_then(|a| reply.rfind('}').map(|b| &reply[a..=b])).ok_or_else(|| short(reply))?;
    let v: Value = serde_json::from_str(json).map_err(|_| short(reply))?;
    let first = if v.is_array() { v.get(0).cloned().unwrap_or(Value::Null) } else { v };
    if first.get("found").and_then(Value::as_bool) == Some(false) {
        return Err(first.get("seen").and_then(Value::as_str).unwrap_or("not that").to_string());
    }
    let num = |k: &str| first.get(k).and_then(Value::as_f64);
    let arr = |k: &str| first.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<_>>());
    let (x, y) = if let (Some(x), Some(y)) = (num("x"), num("y")) {
        (x, y)
    } else if let Some(p) = arr("point_2d").filter(|p| p.len() == 2) {
        (p[0], p[1])
    } else if let Some(b) = arr("bbox_2d").filter(|b| b.len() == 4) {
        ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)
    } else {
        return Err(short(reply));
    };
    if !(0.0..=1000.0).contains(&x) || !(0.0..=1000.0).contains(&y) {
        return Err(format!("a point off the picture ({x}, {y})"));
    }
    let label = first.get("label").and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(Spot { x: x / 1000.0, y: y / 1000.0, label })
}

fn short(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 120 { format!("{}…", s.chars().take(120).collect::<String>()) } else { s.to_string() }
}

/// Where something was found on a window's picture, in its pixels, what's written on it, and the model's replies.
#[derive(Debug, Clone)]
pub struct Found {
    pub x: i32,
    pub y: i32,
    pub label: String,
    pub replies: Vec<String>,
}

/// Finds `what` on a window's picture (RGB, `width`×`height`): one look at the whole window, then a close-up look
/// around that spot at full resolution, since a small icon is a few blurry pixels once the whole window is scaled down
/// (Brave's reload button came out 60 px off on the first look alone). Not seen on the whole window, it's looked for in
/// each quarter of it at full resolution (Brave's lion icon was invisible scaled down). `Err`: not there, or the
/// model failed.
pub async fn find(eyes: &dyn Look, rgb: &[u8], width: u32, height: u32, what: &str) -> Result<Found, String> {
    let whole = encode(rgb, width, height, MAX_SIDE).map_err(|e| format!("{e:#}"))?;
    let first = eyes.look(&whole, &locate_prompt(what)).await.map_err(|e| format!("couldn't look: {e:#}"))?;
    let spot = match parse_spot(&first) {
        Ok(s) => s,
        Err(seen) if whole.width < width => return in_quarters(eyes, rgb, width, height, what, first).await.ok_or(format!("not there (what's there: {seen})")),
        Err(seen) => return Err(format!("not there (what's there: {seen})")),
    };
    let (px, py) = (spot.x * width as f64, spot.y * height as f64);
    let mut found = Found { x: px.round() as i32, y: py.round() as i32, label: spot.label, replies: vec![first] };
    // The close-up: a square around the first spot, a third of the window's shorter side, clamped to the window.
    // A tight square, shown at twice its size: a small model reads small icons better big.
    let side = (width.min(height) / 4).clamp(160, 480).min(width).min(height);
    if whole.width >= width {
        return Ok(found); // nothing was scaled down: the first look was at full detail already
    }
    let x0 = (px as i64 - side as i64 / 2).clamp(0, (width - side) as i64) as u32;
    let y0 = (py as i64 - side as i64 / 2).clamp(0, (height - side) as i64) as u32;
    let close = crop(rgb, width, x0, y0, side, side);
    let Ok(pic) = encode(&enlarge(&close, side, side), side * 2, side * 2, MAX_SIDE) else { return Ok(found) };
    let Ok(second) = eyes.look(&pic, &locate_prompt(what)).await else { return Ok(found) };
    if let Ok(s) = parse_spot(&second) {
        found.x = x0 as i32 + (s.x * side as f64).round() as i32;
        found.y = y0 as i32 + (s.y * side as f64).round() as i32;
        if !s.label.is_empty() {
            found.label = s.label;
        }
    }
    found.replies.push(second);
    Ok(found)
}

/// Looks for `what` in each quarter of the picture (overlapping a little, so nothing sits on a seam), at full detail.
async fn in_quarters(eyes: &dyn Look, rgb: &[u8], width: u32, height: u32, what: &str, first: String) -> Option<Found> {
    let (qw, qh) = ((width * 55 / 100).max(1), (height * 55 / 100).max(1));
    let mut replies = vec![first];
    for (x0, y0) in [(0, 0), (width - qw, 0), (0, height - qh), (width - qw, height - qh)] {
        let pic = encode(&crop(rgb, width, x0, y0, qw, qh), qw, qh, MAX_SIDE).ok()?;
        let reply = eyes.look(&pic, &locate_prompt(what)).await.ok()?;
        let spot = parse_spot(&reply);
        replies.push(reply);
        if let Ok(s) = spot {
            let (x, y) = (x0 as i32 + (s.x * qw as f64).round() as i32, y0 as i32 + (s.y * qh as f64).round() as i32);
            return Some(Found { x, y, label: s.label, replies });
        }
    }
    None
}

/// Twice as wide and high, each pixel doubled.
fn enlarge(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 12) as usize);
    for row in rgb.chunks_exact((w * 3) as usize).take(h as usize) {
        let wide: Vec<u8> = row.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], p[0], p[1], p[2]]).collect();
        out.extend_from_slice(&wide);
        out.extend_from_slice(&wide);
    }
    out
}

/// A rectangle out of RGB pixels (top row first) `width` wide.
pub fn crop(rgb: &[u8], width: u32, x0: u32, y0: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for row in y0..y0 + h {
        let start = ((row * width + x0) * 3) as usize;
        out.extend_from_slice(&rgb[start..start + (w * 3) as usize]);
    }
    out
}

/// The prompt for a question about the picture.
pub fn see_prompt(question: &str) -> String {
    format!(
        "This is a screenshot of one window on the user's PC. {question}\n\
         Answer from what's on it only, plainly and briefly. Quote text exactly as it's written. If it isn't on it, say so. \
         If the question takes for granted something that isn't on it, don't answer yes or no: say what is there \
         instead."
    )
}

#[cfg(windows)]
mod tools {
    use std::time::Duration;

    use super::*;
    use crate::controls::risky;
    use crate::pc::find_window;
    use crate::platform::{self, WinAction, WinInfo};
    use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};
    use serde_json::json;

    const NO_EYES: &str = "her eyes are off: no vision model she may show the screen to (screenshots only go to a local model unless the user allows the cloud)";

    /// The eyes for this call: her own, or the cloud's for the one look you said yes to.
    fn eyes_for(ctx: &Ctx) -> Option<&Eyes> {
        if ctx.use_cloud_eyes { ctx.cloud_eyes.as_ref().map(|(e, _)| e) } else { ctx.eyes.as_ref() }
    }

    /// Her own eyes couldn't make sense of it and there's a stronger pair in the cloud: she asks before a picture of
    /// your window goes there (yes: that one look, the same call again).
    fn ask_cloud(ctx: &Ctx, tool: &str, args: Value, app: &str) -> Value {
        match &ctx.cloud_eyes {
            Some((_, name)) if !ctx.use_cloud_eyes => json!({
                "ask": format!("My eyes can't make sense of {app}. Want me to send a shot of that window to {name} for a look? Just that one."),
                "tool": tool,
                "args": args,
                "done": "",
                "no": "Okay. Nothing leaves your PC. Tell me another way to find it?",
            }),
            _ => Value::Null,
        }
    }

    async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(f).await.expect("blocking task panicked")
    }

    /// The window's picture, brought up first when `raise` (a minimized window has none), once it has settled: a page
    /// still loading looked like "no results" (she gave up on the fifth video while YouTube was still drawing them).
    /// A video playing never settles; after a couple of seconds she looks anyway.
    async fn picture_of(w: &WinInfo, raise: bool) -> Result<(platform::Shot, Picture), String> {
        let id = w.id;
        blocking(move || {
            if raise {
                let _ = platform::act(id, WinAction::Focus);
                std::thread::sleep(Duration::from_millis(350));
            }
            let mut shot = platform::capture(id).map_err(|e| format!("couldn't take its picture: {e:#}"))?;
            let until = std::time::Instant::now() + Duration::from_millis(2500);
            while std::time::Instant::now() < until {
                std::thread::sleep(Duration::from_millis(300));
                let Ok(next) = platform::capture(id) else { break };
                let moving = platform::changed(&shot, &next);
                shot = next;
                if moving < 0.03 {
                    break;
                }
            }
            let pic = encode(&shot.rgb, shot.width, shot.height, MAX_SIDE).map_err(|e| format!("{e:#}"))?;
            Ok((shot, pic))
        })
        .await
    }

    fn window(ctx: &Ctx, name: &str) -> Result<WinInfo, Outcome> {
        let wins: Vec<_> = platform::list_windows().into_iter().filter(|w| w.pid != ctx.own_pid).collect();
        find_window(name, &wins)
    }

    /// What was said to click, as words that could be on it: "the Reject all button" → "reject all".
    fn bare(target: &str) -> String {
        let t = crate::matching::norm(target);
        let t = t.strip_prefix("the ").unwrap_or(&t);
        let mut t = t.to_string();
        for end in [" button", " link", " icon", " tab", " option", " box", " field", " text", " menu"] {
            if let Some(s) = t.strip_suffix(end) {
                t = s.to_string();
            }
        }
        t
    }

    /// The words on the window that are what was asked for: a sure match, or the nth (in reading order) of several
    /// that match as well as each other.
    pub fn by_words<'a>(labels: &'a [platform::ocr::Label], target: &str, nth: Option<usize>) -> Option<&'a platform::ocr::Label> {
        let want = bare(target);
        // One letter or a symbol ("X", "+") matches too much: the window's own close button read as "x".
        if want.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
            return None;
        }
        let ranked = crate::matching::rank(&want, labels, |l| vec![l.text.as_str()], 62);
        match nth {
            Some(n) => {
                let best = ranked.first()?.0;
                if best < 72 {
                    return None;
                }
                let mut same: Vec<&platform::ocr::Label> = ranked.iter().filter(|(s, _)| *s == best).map(|(_, l)| *l).collect();
                same.sort_by_key(|l| (l.rect[1] / 8, l.rect[0]));
                same.get(n.checked_sub(1)?).copied()
            }
            None => crate::matching::sure(&ranked).then(|| ranked[0].1),
        }
    }

    /// Clicks `target` in window `w` by its picture, for windows whose controls can't be read: the words on it first
    /// (Windows' own text reader: exact, quick, nothing leaves the PC), else her eyes.
    pub async fn click_by_sight(ctx: &Ctx, w: &WinInfo, target: &str, nth: Option<usize>) -> Outcome {
        if ctx.dry_run && w.minimized {
            return Outcome::ok(format!("(dry run) would bring up {} and look for \"{target}\"", w.app));
        }
        let (shot, _) = match picture_of(w, !ctx.dry_run).await {
            Ok(p) => p,
            Err(e) => return Outcome::failed(e),
        };
        let (shot, labels) = blocking(move || {
            let labels = platform::ocr::read(&shot).unwrap_or_default();
            (shot, labels)
        })
        .await;
        let mut data = json!({ "window": w.title, "picture": [shot.width, shot.height] });
        let (x, y, seen, how) = match by_words(&labels, target, nth) {
            Some(l) => {
                let (x, y) = l.center();
                data["rect"] = json!(l.rect);
                (x, y, l.text.clone(), "found by its words")
            }
            None => {
                let Some(eyes) = eyes_for(ctx) else { return Outcome::failed(format!("nothing called \"{target}\" among {}'s controls or the words on it, and {NO_EYES}", w.app)) };
                let what = match nth {
                    Some(n) => format!("{target} (number {n} of them, counting left to right, top to bottom)"),
                    None => target.to_string(),
                };
                data["looked_for"] = json!(what);
                let found = match find(eyes.0.as_ref(), &shot.rgb, shot.width, shot.height, &what).await {
                    Ok(f) => f,
                    Err(why) => {
                        let mut args = json!({ "target": target, "window": w.title });
                        if let Some(n) = nth {
                            args["nth"] = json!(n);
                        }
                        data["offer"] = ask_cloud(ctx, "click", args, &w.app);
                        return Outcome::failed(format!("I looked at {} for \"{target}\": {why}", w.app)).with(data);
                    }
                };
                let (x, y) = (shot.left + found.x, shot.top + found.y);
                data["replies"] = json!(found.replies);
                data["rect"] = json!([x - 28, y - 18, x + 28, y + 18]);
                (x, y, if found.label.is_empty() { target.to_string() } else { found.label }, "found by looking")
            }
        };
        data["point"] = json!([x, y]);
        data["label"] = json!(seen);
        if platform::caption_button_at(w.id, x, y) {
            return Outcome::failed(format!("that's {}'s own title-bar button (close, minimize or maximize), not something on its page: to close, minimize or maximize the window use the windows tool; a popup's ✕ is inside the page, so look again", w.app)).with(data);
        }
        // Pressing "Delete" when you said "the third one" isn't a risk to take on a look.
        if risky(&seen) && !risky(target) {
            return Outcome::failed(format!("what's there is \"{seen}\", which could cost something: ask the user first, naming it")).with(data);
        }
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would click \"{seen}\" in {} at ({x}, {y}), {how}", w.app)).with(data);
        }
        let (id, before) = (w.id, w.title.clone());
        let done = blocking(move || {
            // The spot must still be this window's: something may have opened over it since the picture.
            if platform::window_at_not_mine(x, y) != Some(id) {
                return Err("something else is over that spot now".to_string());
            }
            let shot = platform::capture(id).ok();
            platform::click_point(x, y).map_err(|e| format!("{e:#}"))?;
            std::thread::sleep(Duration::from_millis(600));
            let moved = match (shot, platform::capture(id).ok()) {
                (Some(a), Some(b)) => platform::changed(&a, &b),
                _ => 1.0,
            };
            Ok((platform::list_windows().into_iter().find(|x| x.id == id).map(|x| x.title).unwrap_or_default(), moved))
        })
        .await;
        match done {
            Ok((after, moved)) => {
                let titled = after != before && !after.is_empty();
                data["changed"] = json!((moved * 1000.0).round() / 1000.0);
                // Nothing in the window looks any different: the click may have missed (she said "the channel's open"
                // three times about a search box). Said as it is, not as done.
                if !titled && moved < 0.004 {
                    return Outcome::unsure(format!("clicked where \"{seen}\" looked to be in {}, but nothing in the window changed: it may not have done anything (look again before saying it worked)", w.app)).with(data);
                }
                let change = if titled { format!("; the window is now \"{after}\"") } else { String::new() };
                Outcome::ok(format!("clicked \"{seen}\" in {}, {how}{change}", w.app)).with(data)
            }
            Err(e) => Outcome::failed(format!("couldn't click \"{seen}\": {e}")).with(data),
        }
    }

    pub struct See;

    #[async_trait]
    impl Tool for See {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "see".into(),
                description: "Look at a window and answer a question about what's on it: what it shows, what a page, game or \
                              video says, whether something is there, where things are. For what its controls don't tell \
                              (games, launchers, videos, pictures, canvas apps). `window`: the app's name, words of its \
                              title, or \"front\" (the default)."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "window": { "type": "string" }, "question": { "type": "string" } },
                    "required": ["question"]
                }),
            }
        }

        fn risk(&self, _: &Value) -> Risk {
            Risk::Read
        }

        async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
            let question: String = match arg(&args, "question") {
                Ok(q) => q,
                Err(o) => return o,
            };
            let name: String = opt_arg(&args, "window").unwrap_or_else(|| "front".into());
            let Some(eyes) = eyes_for(ctx) else { return Outcome::failed(NO_EYES) };
            let w = match window(ctx, &name) {
                Ok(w) => w,
                Err(o) => return o,
            };
            // Looking doesn't change anything, except bringing up a minimized window to have something to see.
            let (_, pic) = match picture_of(&w, w.minimized && !ctx.dry_run).await {
                Ok(p) => p,
                Err(e) => return Outcome::failed(format!("{}: {e}", w.app)),
            };
            match eyes.0.look(&pic, &see_prompt(&question)).await {
                // Her own eyes saying they can't tell: she can ask for a better look.
                Ok(answer) if muddled(&answer) => {
                    let offer = ask_cloud(ctx, "see", json!({ "window": w.title, "question": question }), &w.app);
                    Outcome::unsure(format!("{} \"{}\": {}", w.app, w.title, answer.trim())).with(json!({ "picture": [pic.width, pic.height], "offer": offer }))
                }
                Ok(answer) => Outcome::ok(format!("{} \"{}\": {}", w.app, w.title, answer.trim())).with(json!({ "picture": [pic.width, pic.height], "rect": w.rect, "label": w.app })),
                Err(e) => Outcome::failed(format!("couldn't look: {e:#}")).with(json!({ "offer": ask_cloud(ctx, "see", json!({ "window": w.title, "question": question }), &w.app) })),
            }
        }
    }
}

#[cfg(windows)]
pub use tools::{See, by_words, click_by_sight};

/// An answer from her eyes that says they couldn't make it out (not "it isn't there", which is an answer).
pub fn muddled(answer: &str) -> bool {
    let a = answer.to_lowercase();
    ["can't tell", "cannot tell", "can't make out", "cannot make out", "unclear", "too small", "blurry", "not legible", "unreadable", "can't read", "cannot read", "unable to determine", "not possible to determine", "hard to tell", "difficult to tell"].iter().any(|w| a.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_in_thousandths() {
        assert_eq!(parse_spot(r#"{"found": true, "x": 500, "y": 250, "label": "Play"}"#).unwrap(), Spot { x: 0.5, y: 0.25, label: "Play".into() });
        // How Qwen-VL answers by habit, and with words around the JSON.
        assert_eq!(parse_spot(r#"Here: {"point_2d": [100, 900]}"#).unwrap().x, 0.1);
        assert_eq!(parse_spot(r#"[{"bbox_2d": [100, 100, 300, 200], "label": "Search"}]"#).unwrap(), Spot { x: 0.2, y: 0.15, label: "Search".into() });
        assert_eq!(parse_spot(r#"{"found": false, "seen": "a login screen"}"#).unwrap_err(), "a login screen");
        assert!(parse_spot("I can't see it").is_err());
        assert!(parse_spot(r#"{"x": 1500, "y": 20}"#).is_err());
    }

    #[test]
    fn pictures_shrink_to_fit() {
        let rgb = vec![200u8; 2560 * 1440 * 3];
        let p = encode(&rgb, 2560, 1440, MAX_SIDE).unwrap();
        assert_eq!((p.width, p.height), (1280, 720));
        let small = encode(&[10u8; 30 * 20 * 3], 30, 20, MAX_SIDE).unwrap();
        assert_eq!((small.width, small.height), (30, 20));
    }

    #[cfg(windows)]
    #[test]
    fn clicks_by_the_words_on_it() {
        use crate::platform::ocr::Label;
        let l = |t: &str, x: i32, y: i32| Label { text: t.into(), rect: [x, y, x + 60, y + 14] };
        let labels = vec![l("Reject all", 10, 500), l("Settings", 90, 500), l("Accept all", 170, 500), l("Reply", 10, 100), l("Reply", 10, 200), l("Ocean Deep Dive", 400, 90)];
        assert_eq!(by_words(&labels, "the Reject all button", None).unwrap().text, "Reject all");
        assert_eq!(by_words(&labels, "ocean deep dive", None).unwrap().rect[0], 400);
        // Two the same: not sure which, unless she's told which one.
        assert!(by_words(&labels, "reply", None).is_none());
        assert_eq!(by_words(&labels, "reply", Some(2)).unwrap().rect[1], 200);
        // A description isn't words on the screen: that's for her eyes.
        assert!(by_words(&labels, "the fifth video", Some(5)).is_none());
    }
}
