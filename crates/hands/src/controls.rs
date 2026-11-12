//! Pressing things inside windows by what they're called: `read_window` lists a window's controls, `click` presses
//! one, `type_in` fills a field. Built on UI Automation (`platform::controls`); matching what you said to a control
//! is plain code here, tested without Windows.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::matching::{norm, score};
use crate::platform::Control;
use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

/// The kind a request names ("search bar" is a field, "the play button" a button), and the words left over.
pub fn kind_in(target: &str) -> (Option<&'static str>, String) {
    let t = format!(" {} ", norm(target));
    const KINDS: &[(&str, &str)] = &[
        (" search bar ", "field"),
        (" search box ", "field"),
        (" search field ", "field"),
        (" address bar ", "field"),
        (" text box ", "field"),
        (" textbox ", "field"),
        (" input ", "field"),
        (" field ", "field"),
        (" box ", "field"),
        (" button ", "button"),
        (" link ", "link"),
        (" tab ", "tab"),
        (" checkbox ", "checkbox"),
        (" check box ", "checkbox"),
        (" toggle ", "checkbox"),
        (" switch ", "checkbox"),
        (" dropdown ", "dropdown"),
        (" menu item ", "menu item"),
        (" slider ", "slider"),
    ];
    for (words, kind) in KINDS {
        if t.contains(words) {
            let rest = t.replacen(words, " ", 1);
            // "search bar" also means the field is about searching.
            let rest = if words.contains("search") { format!("{rest} search") } else { rest };
            return (Some(kind), rest.split_whitespace().filter(|w| !matches!(*w, "the" | "a" | "an" | "on" | "in")).collect::<Vec<_>>().join(" "));
        }
    }
    (None, t.split_whitespace().filter(|w| !matches!(*w, "the" | "a" | "an")).collect::<Vec<_>>().join(" "))
}

/// Words that make a control risky to press without asking.
/// Words that make a control risky wherever they are in its name ("Delete account", "Remove from cart").
const RISKY: &[&str] = &["delete", "remove", "send", "pay", "payment", "buy", "purchase", "checkout", "uninstall", "format", "erase", "wipe", "sign out", "log out", "logout", "unsubscribe", "transfer"];
/// Words that are risky as the control's own action, its first word ("Download", "Install now", "Confirm order"),
/// not when they only describe it: "Downloads", "the download icon", "the resume button for the paused download"
/// were asked about as if they were installs.
const RISKY_ACTION: &[&str] = &["download", "install", "order", "reset", "confirm"];
/// What a risky-sounding first word is describing, not doing: "download manager", "install folder".
const NOT_ACTION: &[&str] = &["icon", "manager", "list", "queue", "page", "tab", "folder", "history", "settings", "menu", "progress", "status", "location", "speed"];

pub fn risky(text: &str) -> bool {
    let n = norm(text);
    let n = n.strip_prefix("the ").unwrap_or(&n);
    let t = format!(" {n} ");
    if RISKY.iter().any(|w| t.contains(&format!(" {w} "))) {
        return true;
    }
    let mut words = n.split(' ');
    let (first, second) = (words.next().unwrap_or(""), words.next());
    RISKY_ACTION.contains(&first) && second.is_none_or(|s| !NOT_ACTION.contains(&s))
}

/// The control meant, or why none could be picked. `nth` counts from 1 among equally good matches, top to bottom.
/// The controls that could be `target`, best first (equal scores keep the window's order).
fn shortlist<'a>(target: &str, controls: &'a [Control]) -> Vec<(u32, &'a Control)> {
    // Its exact name first: "Search or paste link" is that text, not "a link called Search or paste" (every link on
    // the page tied with it, and she gave up).
    let exact: Vec<(u32, &Control)> = controls.iter().filter(|c| c.enabled && !c.name.is_empty() && norm(&c.name) == norm(target)).map(|c| (100, c)).collect();
    if !exact.is_empty() {
        return exact;
    }
    let (kind, words) = kind_in(target);
    // What people call it, and what apps name it: "the chat box" is the field called "Message".
    let alt = also_called(&words);
    let mut scored: Vec<(u32, &Control)> = controls
        .iter()
        .filter(|c| c.enabled)
        .map(|c| {
            let mut s = if words.is_empty() { 0 } else { score(&words, &c.name).max(score(&words, &c.id) / 2) };
            if let Some(alt) = &alt {
                s = s.max(score(alt, &c.name).saturating_sub(10));
            }
            if let Some(k) = kind {
                // The kind helps a control whose name fits, or one with no name ("the search bar"); not any of them.
                if c.kind == k && (s > 0 || c.name.is_empty() || words.is_empty()) {
                    // The right kind of control: a good start on its own ("the search bar" when it's named "Search").
                    s = s.max(40) + 25;
                } else if s > 0 {
                    s = s.saturating_sub(20);
                }
            }
            (s, c)
        })
        .filter(|(s, _)| *s >= 50)
        .collect();
    scored.sort_by_key(|x| std::cmp::Reverse(x.0));
    scored
}

/// Words people use for a control, swapped for the word apps tend to name it with: "chat" → "message", "login" →
/// "sign in". None when there's nothing to swap.
fn also_called(words: &str) -> Option<String> {
    const SAME: &[(&str, &str)] = &[("chat", "message"), ("dm", "message"), ("reply", "message"), ("comment", "add a comment"), ("login", "sign in"), ("log in", "sign in"), ("e-mail", "email"), ("mail", "email"), ("username", "email"), ("find", "search"), ("lookup", "search"), ("url", "address"), ("mute", "mute"), ("volume", "volume")];
    let w = norm(words);
    let swapped = SAME.iter().find(|(said, named)| said != named && (w == *said || w.split(' ').any(|x| x == *said))).map(|(said, named)| w.replace(said, named))?;
    // What's left of how it was described ("chat input box" → "message input box") is the kind, not the name.
    Some(swapped.split(' ').filter(|x| !["box", "input", "field", "bar", "area", "textbox", "the", "a"].contains(x)).collect::<Vec<_>>().join(" "))
}

/// What `pick` chose from, for her journal: the best few, with their scores.
pub fn seen(target: &str, controls: &[Control]) -> Value {
    let list: Vec<Value> = shortlist(target, controls).iter().take(8).map(|(s, c)| json!({ "score": s, "kind": c.kind, "name": c.name, "toggled": c.toggled, "rect": c.rect })).collect();
    json!({ "target": target, "controls": controls.len(), "candidates": list })
}

/// Kinds of controls that do something when pressed; text and images next to them only label them.
const ACTIONABLE: &[&str] = &["button", "split button", "link", "list item", "menu item", "tab", "field", "dropdown", "checkbox", "radio button", "tree item", "item", "slider"];

pub fn pick<'a>(target: &str, nth: Option<usize>, controls: &'a [Control]) -> Result<&'a Control, String> {
    let scored = shortlist(target, controls);
    if scored.is_empty() {
        let named: Vec<String> = controls.iter().filter(|c| !c.name.is_empty()).take(25).map(|c| format!("{} \"{}\"", c.kind, c.name)).collect();
        return Err(format!("nothing called \"{target}\" in that window. What's there: {}", named.join(", ")));
    }
    let best = scored[0].0;
    let mut tied: Vec<&Control> = scored.iter().take_while(|(s, _)| *s == best).map(|(_, c)| *c).collect();
    // The same name on a button and on the text inside it (Steam's "Call of Duty®" was a link, a text and an image):
    // the one that can be pressed.
    if tied.len() > 1 && tied.iter().all(|c| c.name.eq_ignore_ascii_case(&tied[0].name)) && tied.iter().any(|c| ACTIONABLE.contains(&c.kind.as_str())) {
        tied.retain(|c| ACTIONABLE.contains(&c.kind.as_str()));
    }
    if let Some(n) = nth {
        return tied.get(n.saturating_sub(1)).copied().ok_or_else(|| format!("there are only {} \"{target}\"", tied.len()));
    }
    // Several identical names ("Like" under every video) need a number; several different ones need a clearer name.
    if tied.len() > 1 && tied.iter().any(|c| c.name != tied[0].name || c.kind != tied[0].kind) {
        let names: Vec<String> = tied.iter().take(6).map(|c| format!("{} \"{}\"", c.kind, c.name)).collect();
        return Err(format!("more than one control could be \"{target}\": {}. Say which.", names.join(", ")));
    }
    Ok(tied[0])
}

fn describe(c: &Control) -> String {
    let state = match c.toggled {
        Some(true) => " (on)",
        Some(false) => " (off)",
        None => "",
    };
    format!("{} \"{}\"{state}", c.kind, c.name)
}

#[cfg(windows)]
mod tools {
    use super::*;
    use crate::pc::find_window;
    use crate::platform::{self, Pressed};

    async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(f).await.expect("blocking task panicked")
    }

    fn window_of(ctx: &Ctx, name: &str) -> Result<crate::platform::WinInfo, Outcome> {
        let wins: Vec<_> = platform::list_windows().into_iter().filter(|w| w.pid != ctx.own_pid).collect();
        find_window(name, &wins)
    }

    pub struct ReadWindow;

    #[async_trait]
    impl Tool for ReadWindow {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "read_window".into(),
                description: "List the buttons, fields, links, tabs and switches of a window (its real controls, with their \
                              on/off state), to see what can be pressed. `window` is the app's name, words of its title, or \"front\"."
                    .into(),
                parameters: json!({ "type": "object", "properties": { "window": { "type": "string" } }, "required": ["window"] }),
            }
        }

        fn risk(&self, _: &Value) -> Risk {
            Risk::Read
        }

        async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
            let window: String = match arg(&args, "window") {
                Ok(w) => w,
                Err(o) => return o,
            };
            let ctx = ctx.clone();
            blocking(move || {
                let w = match window_of(&ctx, &window) {
                    Ok(w) => w,
                    Err(o) => return o,
                };
                match platform::controls(w.id, 400) {
                    Ok(cs) if cs.iter().filter(|c| !c.name.is_empty()).count() < 4 => {
                        // It draws its own (a game, a launcher, an Electron app): the words on its picture instead.
                        let words: Vec<String> = platform::capture(w.id).ok().and_then(|s| platform::ocr::read(&s).ok()).unwrap_or_default().into_iter().map(|l| l.text).filter(|t| t.chars().any(char::is_alphanumeric)).take(120).collect();
                        if words.is_empty() {
                            Outcome::ok(format!("{} shows no readable controls (it draws its own: use see to look at it, and click still finds things by looking)", w.app))
                        } else {
                            Outcome::ok(format!("{} \"{}\" draws its own controls; the words on it, top to bottom (click finds them by these words): {}", w.app, w.title, words.join(" | ")))
                        }
                    }
                    Ok(cs) => {
                        let list: Vec<String> = cs.iter().filter(|c| !c.name.is_empty()).take(80).map(describe).collect();
                        Outcome::ok(format!("{} \"{}\": {} controls. {}", w.app, w.title, cs.len(), list.join("; ")))
                    }
                    Err(e) => Outcome::failed(format!("couldn't read {}: {e:#}", w.app)),
                }
            })
            .await
        }
    }

    pub struct Click;

    #[async_trait]
    impl Tool for Click {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "click".into(),
                description: "Press a button, link, tab, switch or field in a window, by its name and/or kind (\"the search bar\", \
                              \"Mute\", \"Subscribe button\", \"Settings tab\"). `nth` picks among identical ones (1 = the first, top \
                              to bottom). Switches report their new on/off state. For a field, it puts the cursor in it."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "window": { "type": "string", "description": "App name, words of its title, or \"front\"" },
                        "target": { "type": "string" },
                        "nth": { "type": "integer", "minimum": 1 }
                    },
                    "required": ["window", "target"]
                }),
            }
        }

        fn risk(&self, args: &Value) -> Risk {
            if args.get("target").and_then(Value::as_str).is_some_and(risky) { Risk::Risky } else { Risk::Normal }
        }

        async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
            let (window, target): (String, String) = match (arg(&args, "window"), arg(&args, "target")) {
                (Ok(w), Ok(t)) => (w, t),
                (Err(o), _) | (_, Err(o)) => return o,
            };
            let nth: Option<usize> = opt_arg(&args, "nth");
            let (ctx2, target2) = (ctx.clone(), target.clone());
            // By name first (UI Automation: exact, fast); a window with nothing by that name, or nothing readable at
            // all (games, launchers, canvas apps), is looked at instead.
            let by_name = blocking(move || {
                let (ctx, target) = (ctx2, target2);
                let w = match window_of(&ctx, &window) {
                    Ok(w) => w,
                    Err(o) => return Ok(o),
                };
                let mut cs = platform::controls(w.id, 1500).unwrap_or_default();
                // Nothing in sight by that name: one scrolled out of sight (item 60 of a long list) is brought into
                // view first ("only up to item 7 showing" was a dead end).
                if shortlist(&target, &cs).is_empty() && !ctx.dry_run && platform::scroll_into_view(w.id, &target).unwrap_or(false) {
                    cs = platform::controls(w.id, 1500).unwrap_or_default();
                }
                let data = json!({ "window": w.title, "seen": seen(&target, &cs) });
                if shortlist(&target, &cs).is_empty() {
                    return Err(Box::new(w));
                }
                // Not sure which, or pressing it didn't work: looked at instead (a picture of the window, and where
                // to click decided from it), never a dead end. Only a risky match stops here, to be asked about.
                let c = match pick(&target, nth, &cs) {
                    Ok(c) => c.clone(),
                    Err(why) => {
                        tracing::info!("click by name: {why}; looking instead");
                        return Err(Box::new(w));
                    }
                };
                let data = json!({ "window": w.title, "seen": data["seen"], "chosen": describe(&c), "point": c.center(), "rect": c.rect, "label": c.name });
                let o = press(&ctx, &w, &target, c).with(data);
                if o.status == crate::Status::Failed && !o.summary.contains("could cost something") {
                    tracing::info!("click by name: {}; looking instead", o.summary);
                    return Err(Box::new(w));
                }
                Ok(o)
            })
            .await;
            match by_name {
                Ok(o) => o,
                Err(w) => crate::eyes::click_by_sight(ctx, &w, &target, nth).await,
            }
        }
    }

    /// Presses the control `pick` chose (after the checks), and says what changed.
    fn press(ctx: &Ctx, w: &crate::platform::WinInfo, target: &str, c: Control) -> Outcome {
        // Pressing "Delete account" when you said "account" isn't what you meant to risk.
        if risky(&c.name) && !risky(target) {
            return Outcome::failed(format!("the closest match is {}, which could cost something: ask the user first, naming it", describe(&c)));
        }
        let (cx, cy) = c.center();
        if platform::caption_button_at(w.id, cx, cy) {
            return Outcome::failed(format!("that's {}'s own title-bar button (close, minimize or maximize), not something on its page: to close, minimize or maximize the window use the windows tool; a popup's ✕ is inside the page, so look again", w.app));
        }
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would press {} in {}", describe(&c), w.app));
        }
        let before = w.title.clone();
        let shot = platform::capture(w.id).ok();
        match platform::press_control(w.id, &c) {
            Ok(Pressed::Toggled(on)) => Outcome::ok(format!("switched {} {} in {}", describe(&Control { toggled: None, ..c.clone() }), if on { "on" } else { "off" }, w.app)),
            Ok(Pressed::Focused) => Outcome::ok(format!("the cursor is in {} in {}", describe(&c), w.app)),
            Ok(how) => {
                let how = match how {
                    Pressed::Invoked => "pressed",
                    Pressed::Selected => "selected",
                    Pressed::Expanded => "opened",
                    _ => "clicked",
                };
                std::thread::sleep(std::time::Duration::from_millis(600));
                let after = platform::list_windows().into_iter().find(|x| x.id == w.id).map(|x| x.title).unwrap_or_default();
                let moved = match (shot, platform::capture(w.id).ok()) {
                    (Some(a), Some(b)) => platform::changed(&a, &b),
                    _ => 1.0,
                };
                let titled = after != before && !after.is_empty();
                // Pressed, and nothing in the window looks any different: not something to call done.
                if !titled && moved < 0.004 {
                    return Outcome::unsure(format!("{how} {} in {}, but nothing in the window changed: it may not have done anything (look again before saying it worked)", describe(&c), w.app));
                }
                let change = if titled { format!("; the window is now \"{after}\"") } else { String::new() };
                Outcome::ok(format!("{how} {} in {}{change}", describe(&c), w.app))
            }
            Err(e) => Outcome::failed(format!("couldn't press {}: {e:#}", describe(&c))),
        }
    }

    pub struct TypeIn;

    #[async_trait]
    impl Tool for TypeIn {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "type_in".into(),
                description: "Put text into a field of a window (\"the search bar\", \"Message\"), replacing what's in it, and \
                              press enter after it if `enter` is true (to search, or to send). A slider on a page or in an \
                              app (its volume, brightness, zoom) is set the same way: `field` its name, `text` the number; so is a                               dropdown: `text` the option (\"Large\")."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "window": { "type": "string" },
                        "field": { "type": "string" },
                        "text": { "type": "string" },
                        "enter": { "type": "boolean" }
                    },
                    "required": ["window", "field", "text"]
                }),
            }
        }

        fn risk(&self, args: &Value) -> Risk {
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            if crate::pc::destructive(text) {
                return Risk::NeverSilent;
            }
            // Enter in a message box sends it; in a search or address bar it only searches.
            let field = args.get("field").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let enter = args.get("enter").and_then(Value::as_bool).unwrap_or(false);
            if enter && !["search", "address", "url", "find", "filter"].iter().any(|k| field.contains(k)) { Risk::Risky } else { Risk::Normal }
        }

        async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
            let (window, field, text): (String, String, String) = match (arg(&args, "window"), arg(&args, "field"), arg(&args, "text")) {
                (Ok(w), Ok(f), Ok(t)) => (w, f, t),
                (Err(o), _, _) | (_, Err(o), _) | (_, _, Err(o)) => return o,
            };
            let enter: bool = opt_arg(&args, "enter").unwrap_or(false);
            let ctx = ctx.clone();
            blocking(move || {
                let w = match window_of(&ctx, &window) {
                    Ok(w) => w,
                    Err(o) => return o,
                };
                let mut cs = match platform::controls(w.id, 1500) {
                    Ok(cs) => cs,
                    Err(e) => return Outcome::failed(format!("couldn't read {}: {e:#}", w.app)),
                };
                // A field scrolled out of sight (a chat box low on the page) is brought into view first.
                let in_sight = |cs: &[Control]| cs.iter().any(|c| matches!(c.kind.as_str(), "field" | "dropdown" | "document" | "slider") && norm(&c.name) == norm(&field));
                if !in_sight(&cs) && !ctx.dry_run && platform::scroll_into_view(w.id, &field).unwrap_or(false) {
                    cs = platform::controls(w.id, 1500).unwrap_or(cs);
                }
                // A field is meant even if the name alone would match a button ("Search" button next to the box).
                let fields: Vec<Control> = cs.iter().filter(|c| matches!(c.kind.as_str(), "field" | "dropdown" | "document" | "slider")).cloned().collect();
                let c = match pick(&field, None, &fields).or_else(|_| pick(&format!("{field} field"), None, &cs)) {
                    Ok(c) => c.clone(),
                    Err(why) => return Outcome::failed(why).with(json!({ "window": w.title, "seen": seen(&field, &cs) })),
                };
                let data = json!({ "window": w.title, "seen": seen(&field, &fields), "chosen": describe(&c) });
                fill(&ctx, &w, &c, &text, enter).with(data)
            })
            .await
        }
    }

    /// Types into the field `pick` chose, and says what happened.
    fn fill(ctx: &Ctx, w: &crate::platform::WinInfo, c: &Control, text: &str, enter: bool) -> Outcome {
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would type {} characters into {} in {}{}", text.chars().count(), describe(c), w.app, if enter { " and press enter" } else { "" }));
        }
        // A dropdown is chosen with the keyboard: its window in front first.
        if c.kind == "dropdown" {
            let _ = platform::act(w.id, platform::WinAction::Focus);
        }
        // The field's own Value first (exact, any language, no clipboard); else focus it, select all, type.
        let direct = platform::set_value(w.id, c, text).unwrap_or(false);
        if c.kind == "dropdown" && direct {
            return Outcome::ok(format!("chose \"{}\" in {} in {}", text.trim(), describe(c), w.app));
        }
        if c.kind == "slider" {
            return if direct {
                Outcome::ok(format!("set {} in {} to {}", describe(c), w.app, text.trim()))
            } else {
                Outcome::failed(format!("{} in {} didn't take a number (drag it instead)", describe(c), w.app))
            };
        }
        if !direct {
            let _ = platform::act(w.id, platform::WinAction::Focus);
            if let Err(e) = platform::press_control(w.id, c) {
                return Outcome::failed(format!("couldn't get into {}: {e:#}", describe(c)));
            }
            let _ = platform::press("ctrl+a");
            if let Err(e) = platform::type_text(text) {
                return Outcome::failed(format!("{e:#}"));
            }
        }
        if enter {
            // Enter goes where the caret is: bring the window up and put the caret in the field first.
            let _ = platform::act(w.id, platform::WinAction::Focus);
            let _ = platform::press_control(w.id, c);
            if let Err(e) = platform::press("enter") {
                return Outcome::failed(format!("typed it, but enter didn't go through: {e:#}"));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(if enter { 600 } else { 100 }));
        let after = platform::list_windows().into_iter().find(|x| x.id == w.id).map(|x| x.title).unwrap_or_default();
        let tail = if enter && after != w.title { format!("; the window is now \"{after}\"") } else { String::new() };
        Outcome::ok(format!("typed {} characters into {} in {}{}{tail}", text.chars().count(), describe(c), w.app, if enter { " and pressed enter" } else { "" }))
    }
}

#[cfg(windows)]
pub use tools::{Click, ReadWindow, TypeIn};

#[cfg(test)]
mod tests {
    use super::*;

    fn c(i: usize, name: &str, kind: &str) -> Control {
        Control { index: i, name: name.into(), kind: kind.into(), rect: [0, 0, 10, 10], enabled: true, toggled: None, focused: false, id: String::new() }
    }

    #[test]
    fn a_name_with_a_kind_word_in_it_is_that_name() {
        // Stremio: "Search or paste link" isn't "a link called Search or paste".
        let cs = vec![c(0, "Search or paste link", "text"), c(1, "Board", "link"), c(2, "Discover", "link"), c(3, "Library", "link")];
        assert_eq!(pick("Search or paste link", None, &cs).unwrap().name, "Search or paste link");
        assert!(pick("search bar", None, &cs).is_ok_and(|c| c.name == "Search or paste link"));
    }

    #[test]
    fn the_chat_box_is_the_message_field() {
        let cs = vec![c(0, "Search videos", "field"), c(1, "Message", "field"), c(2, "Send", "button"), c(3, "Email", "field")];
        assert_eq!(pick("chat box", None, &cs).unwrap().name, "Message");
        assert_eq!(pick("the chat", None, &cs).unwrap().name, "Message");
        assert_eq!(pick("Chat input box", None, &cs).unwrap().name, "Message");
    }

    #[test]
        fn a_search_bar_is_a_field() {
        let cs = vec![c(0, "Search", "button"), c(1, "Search", "field"), c(2, "Home", "link")];
        assert_eq!(pick("the search bar", None, &cs).unwrap().kind, "field");
        assert_eq!(pick("search button", None, &cs).unwrap().kind, "button");
    }

    #[test]
    fn identical_names_take_a_number() {
        let cs = vec![c(0, "Like", "button"), c(1, "Like", "button"), c(2, "Like", "button")];
        assert_eq!(pick("like", Some(2), &cs).unwrap().index, 1);
        assert_eq!(pick("like", None, &cs).unwrap().index, 0);
    }

    #[test]
    fn different_names_are_ambiguous() {
        let cs = vec![c(0, "Settings", "button"), c(1, "Settings", "tab"), c(2, "Mute", "checkbox")];
        assert!(pick("settings", None, &cs).is_err());
        assert_eq!(pick("settings tab", None, &cs).unwrap().kind, "tab");
    }

    #[test]
    fn the_pressable_one_of_the_same_name() {
        // Steam's library: the game's name as a link, as the text inside it, and as its picture.
        let cs = vec![c(0, "Call of Duty®", "text"), c(1, "Call of Duty®", "link"), c(2, "Call of Duty®", "image")];
        assert_eq!(pick("Call of Duty", None, &cs).unwrap().kind, "link");
        let cs = vec![c(0, "Call of Duty®", "text"), c(1, "Call of Duty®", "link"), c(2, "Call of Duty®", "button")];
        assert!(pick("Call of Duty", None, &cs).is_err(), "a link and a button: which one is a real question");
    }

    #[test]
    fn nothing_there_says_what_is() {
        let cs = vec![c(0, "Mute", "checkbox"), c(1, "Deafen", "checkbox")];
        let e = pick("camera", None, &cs).unwrap_err();
        assert!(e.contains("Mute") && e.contains("Deafen"), "{e}");
    }

    #[test]
    fn risky_words() {
        assert!(risky("Delete account"));
        assert!(risky("Send message"));
        assert!(!risky("Mute"));
        assert!(!risky("Subscribe"));
        assert!(risky("Download"));
        assert!(risky("Install now"));
        assert!(risky("Confirm purchase"));
        assert!(!risky("Downloads"));
        assert!(!risky("the download icon at bottom left"));
        assert!(!risky("the resume button (play icon) for the paused download"));
        assert!(!risky("PayPal"));
    }
}
