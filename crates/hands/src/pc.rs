//! Her PC tools: windows, opening things, searching sites, media, volume, keys and typing. Each one checks what it
//! did and says so; none claims more than it saw.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::matching::{rank, score, sure};
use crate::platform::{self, AppEntry, WinAction, WinInfo};
use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

/// Runs blocking OS calls off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("blocking task panicked")
}

/// Windows that aren't hers.
fn others(ctx: &Ctx) -> Vec<WinInfo> {
    platform::list_windows().into_iter().filter(|w| w.pid != ctx.own_pid).collect()
}

fn describe_windows(wins: &[WinInfo]) -> String {
    wins.iter()
        .map(|w| format!("\"{}\" ({}{}{})", short(&w.title, 70), w.app, if w.foreground { ", in front" } else { "" }, if w.minimized { ", minimized" } else { "" }))
        .collect::<Vec<_>>()
        .join("; ")
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// Finds the window you meant: "front" for the one in front, else by app name, exe name or words of its title.
pub fn find_window(target: &str, wins: &[WinInfo]) -> Result<WinInfo, Outcome> {
    let t = target.trim().to_lowercase();
    if matches!(t.as_str(), "" | "front" | "current" | "this" | "this window" | "it" | "the window in front" | "active") {
        return wins.iter().find(|w| w.foreground).cloned().ok_or_else(|| Outcome::failed("no window is in front"));
    }
    let ranked = rank(target, wins, |w| vec![w.app.as_str(), w.exe.as_str(), w.title.as_str()], 50);
    // Title words count less than the app's name: "close spotify" shouldn't pick a browser tab titled "Spotify – …"
    // over Spotify itself, unless that's all there is.
    let mut ranked: Vec<(u32, &WinInfo)> = ranked
        .into_iter()
        .map(|(s, w)| {
            let by_app = score(target, &w.app).max(score(target, &w.exe));
            (if by_app >= s { s } else { s.saturating_sub(15) }, w)
        })
        .collect();
    ranked.sort_by_key(|x| std::cmp::Reverse(x.0));
    if ranked.is_empty() {
        return Err(Outcome::failed(format!("no open window matches \"{target}\". Open windows: {}", describe_windows(wins))));
    }
    // One candidate is the one. Several windows of the same app (two Brave windows): the frontmost is meant
    // (`wins` is front to back, and the sort above is stable).
    let top = ranked[0].0;
    let same_app = ranked.iter().all(|(_, w)| w.exe == ranked[0].1.exe);
    let tied_same_app = ranked.iter().take_while(|(s, _)| *s == top).all(|(_, w)| w.exe == ranked[0].1.exe);
    if ranked.len() == 1 || sure(&ranked) || same_app || (top >= 72 && tied_same_app) {
        return Ok(ranked[0].1.clone());
    }
    // Close between two apps, and one of them is the window in front: that's the one being worked in ("the Gym
    // page" was the Edge window in front, not a Brave tab of a search for "Gym").
    if let Some((_, w)) = ranked.iter().take_while(|(s, _)| *s + 10 > top).find(|(s, w)| w.foreground && *s >= 62) {
        return Ok((*w).clone());
    }
    let names: Vec<String> = ranked.iter().take(5).map(|(_, w)| format!("\"{}\" ({})", short(&w.title, 60), w.app)).collect();
    Err(Outcome::failed(format!("more than one window could be \"{target}\": {}. Say which (use words from its title).", names.join(", "))))
}

/// Windows she minimized with "minimize all", to bring exactly those back.
static MINIMIZED: Mutex<Vec<isize>> = Mutex::new(Vec::new());

pub struct Windows;

#[async_trait]
impl Tool for Windows {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "windows".into(),
            description: "List the open windows, or focus (bring to front), minimize, maximize, restore or close one, or minimize / \
                          restore all of them, or put one on the left / right / top / bottom half of its screen (two side \
                          by side: one left, the other right), or keep one on top of the rest (pin_on_top, unpin). `target` \
                          is the app's name or words from the window's title, or \"front\" for the window in front. Closing \
                          works like the window's own close button (the app may ask to save)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "focus", "minimize", "maximize", "restore", "close", "force_close", "minimize_all", "restore_all", "snap_left", "snap_right", "snap_top", "snap_bottom", "pin_on_top", "unpin"] },
                    "target": { "type": "string", "description": "Which window, for one-window actions." }
                },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        match args.get("action").and_then(Value::as_str) {
            Some("list") => Risk::Read,
            // Anything unsaved in it is lost.
            Some("force_close") => Risk::Risky,
            _ => Risk::Normal,
        }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        let target: String = opt_arg(&args, "target").unwrap_or_default();
        let ctx = ctx.clone();
        blocking(move || {
            let wins = others(&ctx);
            match action.as_str() {
                "list" => Outcome::ok(format!("{} windows, front to back: {}", wins.len(), describe_windows(&wins))).with(json!(wins)),
                "minimize_all" => {
                    let mut done = Vec::new();
                    for w in wins.iter().filter(|w| !w.minimized) {
                        if platform::act(w.id, WinAction::Minimize).is_ok() {
                            done.push(w.id);
                        }
                    }
                    let n = done.len();
                    *MINIMIZED.lock().unwrap() = done;
                    let left = others(&ctx).into_iter().filter(|w| !w.minimized).count();
                    if left == 0 { Outcome::ok(format!("minimized {n} windows")) } else { Outcome::unsure(format!("minimized {n} windows; {left} didn't go down")) }
                }
                "restore_all" => {
                    let mut ids = std::mem::take(&mut *MINIMIZED.lock().unwrap());
                    if ids.is_empty() {
                        ids = wins.iter().filter(|w| w.minimized).map(|w| w.id).collect();
                    }
                    let n = ids.iter().filter(|&&id| platform::act(id, WinAction::Restore).is_ok()).count();
                    if n == 0 { Outcome::failed("no minimized windows to bring back") } else { Outcome::ok(format!("brought back {n} windows")) }
                }
                one if one.starts_with("snap_") || one == "pin_on_top" || one == "unpin" => {
                    let place = match one {
                        "snap_left" => platform::Place::Left,
                        "snap_right" => platform::Place::Right,
                        "snap_top" => platform::Place::Top,
                        "snap_bottom" => platform::Place::Bottom,
                        "pin_on_top" => platform::Place::OnTop,
                        _ => platform::Place::NotOnTop,
                    };
                    let w = match find_window(&target, &wins) {
                        Ok(w) => w,
                        Err(o) => return o,
                    };
                    if ctx.dry_run {
                        return Outcome::ok(format!("(dry run) would {one} \"{}\" ({})", w.title, w.app));
                    }
                    match platform::place(w.id, place) {
                        Ok(seen) => Outcome::ok(format!("{} \"{}\": {seen}", w.app, short(&w.title, 80))),
                        Err(e) => Outcome::failed(format!("{one} {}: {e:#}", w.app)),
                    }
                }
                one => {
                    let act = match one {
                        "focus" => WinAction::Focus,
                        "minimize" => WinAction::Minimize,
                        "maximize" => WinAction::Maximize,
                        "restore" => WinAction::Restore,
                        "close" => WinAction::Close,
                        "force_close" => WinAction::ForceClose,
                        other => return Outcome::failed(format!("unknown action \"{other}\"")),
                    };
                    let w = match find_window(&target, &wins) {
                        Ok(w) => w,
                        Err(o) if one == "focus" && o.summary.starts_with("no open window matches") => {
                            // Closed app: open it; don't poke around other windows forever.
                            return Outcome::failed(format!(
                                "\"{target}\" isn't open. Open it with the open tool (name \"{target}\") first. If open \
                                 says it isn't installed, tell them that — don't explore other windows looking for it."
                            ));
                        }
                        Err(o) => return o,
                    };
                    if ctx.dry_run {
                        return Outcome::ok(format!("(dry run) would {one} \"{}\" ({})", w.title, w.app));
                    }
                    match platform::act(w.id, act) {
                        Ok(seen) => Outcome::ok(format!("{} \"{}\": {seen}", w.app, short(&w.title, 80))),
                        // Still open after being asked to close: she offers to force quit it (a yes from you does it).
                        Err(e) if one == "close" && format!("{e:#}").contains("still open") => {
                            let mut o = Outcome::failed(format!("close {} \"{}\": {e:#}", w.app, short(&w.title, 80)));
                            o.data = json!({ "offer": { "ask": format!("{} won't close. Anything unsaved in it would be lost. Force quit it?", w.app), "tool": "windows", "args": { "action": "force_close", "target": w.title }, "done": format!("Force quit. {} is gone.", w.app), "no": "Okay. It stays open." } });
                            o
                        }
                        Err(e) => Outcome::failed(format!("{one} {} \"{}\": {e:#}", w.app, short(&w.title, 80))),
                    }
                }
            }
        })
        .await
    }
}

/// The Start menu's apps, read once and refreshed every few minutes (new installs show up).
static APPS: Mutex<Option<(Instant, Vec<AppEntry>)>> = Mutex::new(None);

fn app_list() -> Vec<AppEntry> {
    let mut cache = APPS.lock().unwrap();
    if let Some((at, list)) = cache.as_ref()
        && at.elapsed() < Duration::from_secs(300)
    {
        return list.clone();
    }
    let list = platform::apps().unwrap_or_else(|e| {
        tracing::warn!("apps: {e:#}");
        Vec::new()
    });
    *cache = Some((Instant::now(), list.clone()));
    list
}

/// Places with a name of their own.
fn known_place(q: &str) -> Option<(&'static str, &'static str)> {
    let q = crate::matching::norm(q);
    let q = q.strip_prefix("my ").unwrap_or(&q).trim_end_matches(" folder").to_string();
    Some(match q.as_str() {
        "downloads" | "download" => ("Downloads", "shell:Downloads"),
        "documents" | "docs" => ("Documents", "shell:Personal"),
        "desktop" => ("Desktop", "shell:Desktop"),
        "pictures" | "photos" | "images" => ("Pictures", "shell:My Pictures"),
        "music" => ("Music", "shell:My Music"),
        "videos" => ("Videos", "shell:My Video"),
        "recycle bin" | "trash" | "bin" => ("Recycle Bin", "shell:RecycleBinFolder"),
        "this pc" | "my computer" | "computer" => ("This PC", "shell:MyComputerFolder"),
        "file explorer" | "explorer" | "files" => ("File Explorer", "shell:MyComputerFolder"),
        "screenshots" => ("Screenshots", "shell:Screenshots"),
        "settings" | "windows settings" => ("Settings", "ms-settings:"),
        // Windows' own settings pages, by what people call them.
        "bluetooth" | "bluetooth settings" | "devices" => ("Bluetooth settings", "ms-settings:bluetooth"),
        "sound settings" | "sound" | "audio settings" => ("Sound settings", "ms-settings:sound"),
        "display settings" | "display" | "screen settings" | "resolution" => ("Display settings", "ms-settings:display"),
        "windows update" | "windows updates" | "updates" | "update settings" => ("Windows Update", "ms-settings:windowsupdate"),
        "wifi" | "wi-fi" | "wifi settings" | "network settings" | "internet settings" => ("Wi-Fi settings", "ms-settings:network-wifi"),
        "notifications" | "notification settings" => ("Notification settings", "ms-settings:notifications"),
        "apps" | "installed apps" | "apps and features" | "uninstall a program" => ("Installed apps", "ms-settings:appsfeatures"),
        "storage" | "storage settings" | "disk cleanup" => ("Storage settings", "ms-settings:storagesense"),
        "battery" | "battery settings" | "power settings" | "power" => ("Power & battery", "ms-settings:powersleep"),
        "mouse" | "mouse settings" | "touchpad" => ("Mouse settings", "ms-settings:mousetouchpad"),
        "background" | "wallpaper" | "personalization" => ("Background", "ms-settings:personalization-background"),
        "colors" | "dark mode" | "light mode" | "theme" => ("Colors (dark and light mode)", "ms-settings:colors"),
        "night light" => ("Night light", "ms-settings:nightlight"),
        "microphone settings" | "mic settings" => ("Microphone", "ms-settings:privacy-microphone"),
        "default apps" => ("Default apps", "ms-settings:defaultapps"),
        "keyboard settings" | "language settings" => ("Language & region", "ms-settings:regionlanguage"),
        "snipping tool" | "snip" | "screen snip" => ("Snipping Tool", "ms-screenclip:"),
        _ => return None,
    })
}

fn looks_like_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://") || s.starts_with("https://") || (!s.contains(' ') && s.contains('.') && s.rsplit('.').next().is_some_and(|tld| (2..=6).contains(&tld.len()) && tld.chars().all(|c| c.is_ascii_alphabetic())))
}

/// Opens a file, folder, app id or link the way double-clicking it would. Links go to Windows' own "open" (to your
/// default browser): handed to explorer.exe, a link with `?` in it can open a folder instead.
pub(crate) fn shell_open(target: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    if target.contains("://") || ["spotify:", "steam:", "ms-settings:", "ms-screenclip:", "mailto:"].iter().any(|s| target.starts_with(s)) {
        use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        use windows::core::{HSTRING, w};
        let done = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(target), None, None, SW_SHOWNORMAL) };
        // Above 32 is success (Win32's old convention).
        return if done.0 as isize > 32 { Ok(()) } else { Err(std::io::Error::other(format!("Windows couldn't open it (code {})", done.0 as isize))) };
    }
    std::process::Command::new("explorer.exe").arg(target).spawn().map(|_| ())
}

pub struct Open;

#[async_trait]
impl Tool for Open {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "open".into(),
            description: "Open an app or game by name (anything in the Start menu, Store apps included, and every Steam game, \
                          also by what people call it: \"Warzone\" is Call of Duty), a folder (downloads, \
                          documents, desktop, pictures, music, videos, screenshots, recycle bin, this PC, file explorer), \
                          Windows settings, or a website (a URL or domain). If the app already has a window, it's brought to \
                          the front. If the name is ambiguous, the candidates come back: call again with the exact one."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "App name, folder, or URL" } },
                "required": ["name"]
            }),
        }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let name: String = match arg(&args, "name") {
            Ok(n) => n,
            Err(o) => return o,
        };
        let ctx = ctx.clone();
        blocking(move || open_blocking(&name, &ctx)).await
    }
}

fn open_blocking(name: &str, ctx: &Ctx) -> Outcome {
    if let Some((label, target)) = known_place(name) {
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would open {label}"));
        }
        return match shell_open(target) {
            Ok(()) => Outcome::ok(format!("opened {label}")),
            Err(e) => Outcome::failed(format!("couldn't open {label}: {e}")),
        };
    }
    if looks_like_url(name) {
        let url = if name.starts_with("http") { name.to_string() } else { format!("https://{}", name.trim()) };
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would open {url}"));
        }
        return match shell_open(&url) {
            Ok(()) => Outcome::ok(format!("opened {url} in the default browser")),
            Err(e) => Outcome::failed(format!("couldn't open {url}: {e}")),
        };
    }
    // Already open? Bring it forward instead of starting a second one.
    let wins = others(ctx);
    let wanted = crate::library::alias(name);
    let mut open_ranked = rank(name, &wins, |w| vec![w.app.as_str(), w.exe.as_str()], 72);
    if open_ranked.is_empty() && wanted != name {
        open_ranked = rank(&wanted, &wins, |w| vec![w.app.as_str(), w.exe.as_str(), w.title.as_str()], 72);
    }
    if let Some((_, w)) = open_ranked.first() {
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) {} is already open; would bring it to the front", w.app));
        }
        return match platform::act(w.id, WinAction::Focus) {
            Ok(_) => Outcome::ok(format!("{} was already open; brought it to the front", w.app)),
            Err(e) => Outcome::unsure(format!("{} is already open but couldn't come to the front: {e:#}", w.app)),
        };
    }
    // The Start menu (Store and Xbox apps included) and every store's library and the desktop's shortcuts, as one
    // list. A library entry's id is what to open: a launcher's URL or a shortcut file.
    let mut apps = app_list();
    let mut from: Vec<(String, &'static str)> = Vec::new();
    for e in crate::library::entries() {
        if !apps.iter().any(|a| crate::matching::norm(&a.name) == crate::matching::norm(&e.name)) {
            from.push((e.launch.clone(), e.from));
            apps.push(AppEntry { name: e.name, id: e.launch });
        }
    }
    let ranked = rank(&wanted, &apps, |a| vec![a.name.as_str()], 50);
    // Nothing by that name, but one close to it: she asks ("Did you mean Stremio?") instead of a dead end.
    if ranked.is_empty()
        && let Some(near) = crate::matching::closest(&wanted, &apps, |a| vec![a.name.as_str()])
    {
        let mut o = Outcome::failed(format!("no app called \"{name}\"; the closest is {}", near.name));
        o.data = json!({ "did_you_mean": near.name, "tool": "open" });
        return o;
    }
    if ranked.is_empty() {
        // Her libraries are read already: "on Steam" and not in Steam's library means it isn't installed (she spent
        // 160 s clicking through Steam's window for a Warzone that wasn't there).
        return Outcome::failed(format!("\"{name}\" isn't installed through the Start menu, Steam, Epic, GOG, Ubisoft or a desktop shortcut (their libraries are read already: don't look for it in their windows). If they said which store, it isn't installed there: say so and offer its store page. Only a launcher she can't read (Battle.net, EA app, Riot, Rockstar) is worth opening to look in"));
    }
    if !sure(&ranked) {
        let names: Vec<&str> = ranked.iter().take(6).map(|(_, a)| a.name.as_str()).collect();
        return Outcome::failed(format!("several apps could be \"{name}\": {}. Call open again with the exact name.", names.join(", ")));
    }
    let app = ranked[0].1.clone();
    if ctx.dry_run {
        return Outcome::ok(format!("(dry run) would start {}", app.name));
    }
    // From a store's library: its launcher starts it (updates and all), which takes longer than a window would wait.
    if let Some((_, store)) = from.iter().find(|(id, _)| *id == app.id)
        && *store != "desktop"
    {
        return match shell_open(&app.id) {
            Ok(()) => Outcome::ok(format!("asked {store} to start {} (games take a moment to show a window)", app.name)),
            Err(e) => Outcome::failed(format!("couldn't ask {store} to start {}: {e}", app.name)),
        };
    }
    // A shortcut on the desktop: opened like a double-click.
    let launched = if from.iter().any(|(id, _)| *id == app.id) { shell_open(&app.id).map_err(anyhow::Error::from) } else { platform::launch(&app.id) };
    if let Err(e) = launched {
        return Outcome::failed(format!("{}: {e:#}", app.name));
    }
    let before: Vec<isize> = wins.iter().map(|w| w.id).collect();
    let appeared = platform::wait_for_window(|w| !before.contains(&w.id) && (score(&app.name, &w.app) >= 50 || score(&app.name, &w.title) >= 50 || score(name, &w.app) >= 50), Duration::from_secs(10));
    match appeared {
        Some(w) => Outcome::ok(format!("started {}; its window is up: \"{}\"", app.name, short(&past_splash(&w), 80))),
        None => Outcome::unsure(format!("started {}; no window yet (it may still be loading, or it opened in the tray)", app.name)),
    }
}

/// What a window just opened is called once its splash is over. Many apps sit on a "Sign in" or "Loading" window for a few
/// seconds before the real one (she tried signing in to Steam while it was still signing itself in). Up to 12 s; if it's
/// still there it's a real sign-in, and she says so.
fn past_splash(w: &platform::WinInfo) -> String {
    let passing = |t: &str| {
        let t = t.to_lowercase();
        ["sign in", "sign-in", "log in", "login", "loading", "starting", "updating", "please wait"].iter().any(|s| t.contains(s))
    };
    if !passing(&w.title) {
        return w.title.clone();
    }
    let end = std::time::Instant::now() + Duration::from_secs(12);
    while std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(500));
        if let Some(now) = platform::list_windows().into_iter().find(|x| x.exe == w.exe && !x.title.trim().is_empty() && !passing(&x.title)) {
            return now.title;
        }
    }
    w.title.clone()
}

pub struct Search;

#[async_trait]
impl Tool for Search {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "search".into(),
            description: "Open a search on a site in the browser: youtube, youtube_music, google, spotify, reddit, github, \
                          wikipedia, amazon, twitch, steam. This only opens the results page; to play something, use \
                          play_music or click on the page afterwards."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "site": { "type": "string", "enum": ["youtube", "youtube_music", "google", "spotify", "reddit", "github", "wikipedia", "amazon", "twitch", "steam"] }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let query: String = match arg(&args, "query") {
            Ok(q) => q,
            Err(o) => return o,
        };
        let site: String = opt_arg(&args, "site").unwrap_or_else(|| "google".into());
        let q = urlencode(&query);
        let url = match site.to_lowercase().as_str() {
            "youtube" => format!("https://www.youtube.com/results?search_query={q}"),
            "youtube_music" => format!("https://music.youtube.com/search?q={q}"),
            // Web search, not spotify: — that URI pops a Store dialog when Spotify isn't installed.
            "spotify" => format!("https://open.spotify.com/search/{q}"),
            "reddit" => format!("https://www.reddit.com/search/?q={q}"),
            "github" => format!("https://github.com/search?q={q}"),
            "wikipedia" => format!("https://en.wikipedia.org/w/index.php?search={q}"),
            "amazon" => format!("https://www.amazon.com/s?k={q}"),
            "twitch" => format!("https://www.twitch.tv/search?term={q}"),
            "steam" => format!("https://store.steampowered.com/search/?term={q}"),
            "google" => format!("https://www.google.com/search?q={q}"),
            // Never open Google under a fake site name ("stremio", "loaded.com"): that looked like success and wasn't.
            other => {
                return Outcome::failed(format!(
                    "search only works for youtube, youtube_music, google, spotify, reddit, github, wikipedia, amazon, twitch, steam — not \"{other}\". \
                     For an app or other site: open or focus it, then type_in or click its search field"
                ));
            }
        };
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would open {url}"));
        }
        match shell_open(&url) {
            Ok(()) => Outcome::ok(format!("opened the {site} search for \"{query}\"")),
            Err(e) => Outcome::failed(format!("couldn't open the browser: {e}")),
        }
    }
}

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub struct Media;

#[async_trait]
impl Tool for Media {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "media".into(),
            description: "Control what's playing in any app (YouTube in the browser, Spotify, YouTube Music…): play, pause, \
                          toggle, next, previous; or `status` for what's playing and paused right now."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "action": { "type": "string", "enum": ["status", "play", "pause", "toggle", "next", "previous"] } },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        if args.get("action").and_then(Value::as_str) == Some("status") { Risk::Read } else { Risk::Normal }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        let dry = ctx.dry_run;
        blocking(move || {
            if action == "status" {
                return match platform::now_playing() {
                    Ok(list) if list.is_empty() => Outcome::ok("nothing is playing or paused in any app"),
                    Ok(list) => {
                        let s: Vec<String> = list.iter().map(|n| format!("{} \"{}\"{} in {}", if n.playing { "playing" } else { "paused" }, n.title, if n.artist.is_empty() { String::new() } else { format!(" by {}", n.artist) }, n.app)).collect();
                        Outcome::ok(s.join("; ")).with(json!(list))
                    }
                    Err(e) => Outcome::failed(format!("couldn't read the media sessions: {e:#}")),
                };
            }
            if dry {
                return Outcome::ok(format!("(dry run) would {action}"));
            }
            match platform::media(&action) {
                Ok(n) => {
                    let state = if n.playing { "playing" } else { "paused" };
                    let ok = match action.as_str() {
                        "pause" => !n.playing,
                        "play" => n.playing,
                        _ => true,
                    };
                    let what = format!("{state}: \"{}\"{} in {}", n.title, if n.artist.is_empty() { String::new() } else { format!(" by {}", n.artist) }, n.app);
                    if ok { Outcome::ok(format!("{action} done; now {what}")) } else { Outcome::unsure(format!("asked to {action}, but the player says {what}")) }
                }
                Err(e) => Outcome::failed(format!("{action}: {e:#}")),
            }
        })
        .await
    }
}

pub struct Volume;

#[async_trait]
impl Tool for Volume {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "volume".into(),
            description: "The speakers' volume (Windows' master volume): get it, set it to a percent, turn it up or down (by 10 \
                          unless `by` says otherwise), mute or unmute the speakers. Never for \"mute me\" or a microphone: \
                          that's the user's mic, the discord tool."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["get", "set", "up", "down", "mute", "unmute"] },
                    "percent": { "type": "integer", "minimum": 0, "maximum": 100 },
                    "by": { "type": "integer", "minimum": 1, "maximum": 100 }
                },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        if args.get("action").and_then(Value::as_str) == Some("get") { Risk::Read } else { Risk::Normal }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        let percent: Option<i64> = opt_arg(&args, "percent");
        let by: i64 = opt_arg(&args, "by").unwrap_or(10);
        let dry = ctx.dry_run;
        blocking(move || {
            let now = match platform::volume() {
                Ok(v) => v as i64,
                Err(e) => return Outcome::failed(format!("couldn't read the volume: {e:#}")),
            };
            let muted = platform::mute().unwrap_or(false);
            let target = match action.as_str() {
                "get" => return Outcome::ok(format!("volume {now}%{}", if muted { ", muted" } else { "" })),
                "set" => match percent {
                    Some(p) => p,
                    None => return Outcome::failed("`set` needs `percent`"),
                },
                "up" => now + by,
                "down" => now - by,
                "mute" | "unmute" => {
                    if dry {
                        return Outcome::ok(format!("(dry run) would {action}"));
                    }
                    let want = action == "mute";
                    if muted == want {
                        return Outcome::ok(format!("the speakers were already {}", if want { "muted" } else { "on" }));
                    }
                    return match platform::set_mute(want) {
                        Ok(m) if m == want => Outcome::ok(if want { "speakers muted".to_string() } else { format!("speakers unmuted, volume {now}%") }),
                        Ok(_) => Outcome::failed("Windows didn't change the mute"),
                        Err(e) => Outcome::failed(format!("{e:#}")),
                    };
                }
                other => return Outcome::failed(format!("unknown action \"{other}\"")),
            };
            let target = target.clamp(0, 100) as u32;
            if dry {
                return Outcome::ok(format!("(dry run) would set the volume from {now}% to {target}%"));
            }
            match platform::set_volume(target) {
                Ok(v) => Outcome::ok(format!("volume {now}% → {v}%")),
                Err(e) => Outcome::failed(format!("{e:#}")),
            }
        })
        .await
    }
}

const BROWSERS: &[&str] = &["brave", "chrome", "msedge", "firefox", "opera", "vivaldi", "arc", "zen", "librewolf", "waterfox"];

/// Browser tabs and pages, in the browser you were last in: closing "this tab" must never close the window.
pub struct Tab;

#[async_trait]
impl Tool for Tab {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "tab".into(),
            description: "Browser tabs and pages, in the browser in front (or the most recent one): close this tab, reopen the \
                          last closed tab, new tab, next / previous tab, back, forward, reload, or (only when the user says \
                          \"tab\") switch to the tab whose \
                          title or site has the words in `to` (\"my YouTube tab\": action switch, to \"YouTube\"). Use this \
                          for anything about a tab; it never closes the browser window."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["close", "reopen", "new", "next", "previous", "back", "forward", "reload", "switch"] },
                    "to": { "type": "string", "description": "For switch: words of the tab's title or site" }
                },
                "required": ["action"]
            }),
        }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        if action == "switch" {
            let to: String = match arg(&args, "to") {
                Ok(t) => t,
                Err(o) => return o,
            };
            return switch_tab(ctx.clone(), to).await;
        }
        let keys = match action.as_str() {
            "close" => "ctrl+w",
            "reopen" => "ctrl+shift+t",
            "new" => "ctrl+t",
            "next" => "ctrl+tab",
            "previous" => "ctrl+shift+tab",
            "back" => "alt+left",
            "forward" => "alt+right",
            "reload" => "f5",
            other => return Outcome::failed(format!("unknown tab action \"{other}\"")),
        };
        let ctx = ctx.clone();
        blocking(move || {
            // Front to back: the first browser is the one you were last in.
            let Some(w) = others(&ctx).into_iter().find(|w| BROWSERS.contains(&w.exe.as_str()) && !w.minimized) else {
                return Outcome::failed("no browser window is open (or they're all minimized)");
            };
            if ctx.dry_run {
                return Outcome::ok(format!("(dry run) would press {keys} in {} (\"{}\")", w.app, short(&w.title, 60)));
            }
            if !w.foreground
                && let Err(e) = platform::act(w.id, WinAction::Focus)
            {
                return Outcome::failed(format!("couldn't bring {} to the front: {e:#}", w.app));
            }
            if let Err(e) = platform::press(keys) {
                return Outcome::failed(format!("{e:#}"));
            }
            std::thread::sleep(Duration::from_millis(350));
            let now = platform::list_windows().into_iter().find(|x| x.id == w.id).map(|x| x.title);
            match now {
                Some(t) if t != w.title => Outcome::ok(format!("{action} done in {}; now showing \"{}\"", w.app, short(&t, 80))),
                Some(_) => Outcome::ok(format!("pressed {keys} in {} (the title didn't change)", w.app)),
                None => Outcome::ok(format!("{action} done; that was the last tab, so {} closed", w.app)),
            }
        })
        .await
    }
}

/// To the tab whose title has `to`'s words, in the browser in front: ctrl+tab through them, reading the window's
/// title each time, until it's there or she's back where she started (any browser: the title is the tab's).
async fn switch_tab(ctx: Ctx, to: String) -> Outcome {
    blocking(move || {
        let Some(w) = others(&ctx).into_iter().find(|w| BROWSERS.contains(&w.exe.as_str()) && !w.minimized) else {
            return Outcome::failed("no browser window is open (or they're all minimized)");
        };
        let words: Vec<String> = to.to_lowercase().split_whitespace().filter(|w| !matches!(*w, "tab" | "the" | "my" | "page")).map(String::from).collect();
        let has = |title: &str| {
            let t = title.to_lowercase();
            !words.is_empty() && words.iter().all(|w| t.contains(w.as_str()))
        };
        if has(&w.title) {
            return Outcome::ok(format!("that tab is already the one showing in {}: \"{}\"", w.app, short(&w.title, 80)));
        }
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would go through {}'s tabs to the one with \"{to}\"", w.app));
        }
        if !w.foreground
            && let Err(e) = platform::act(w.id, WinAction::Focus)
        {
            return Outcome::failed(format!("couldn't bring {} to the front: {e:#}", w.app));
        }
        let title = |id: isize| platform::list_windows().into_iter().find(|x| x.id == id).map(|x| x.title).unwrap_or_default();
        let start = w.title.clone();
        let mut seen = vec![start.clone()];
        for _ in 0..40 {
            if platform::press("ctrl+tab").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
            let now = title(w.id);
            if has(&now) {
                return Outcome::ok(format!("switched to the tab \"{}\" in {}", short(&now, 80), w.app));
            }
            if now == start {
                break;
            }
            seen.push(now);
        }
        seen.dedup();
        Outcome::failed(format!("no tab in {} has \"{to}\" in its title; the tabs are: {}", w.app, seen.iter().map(|t| format!("\"{}\"", short(t, 50))).collect::<Vec<_>>().join(", ")))
    })
    .await
}

pub struct Keys;

#[async_trait]
impl Tool for Keys {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "keys".into(),
            description: "Press a key or a shortcut (\"enter\", \"ctrl+shift+m\", \"alt+tab\", \"f5\", \"media_play_pause\") in a \
                          window: `target` brings that window to the front first (omit it for the window already in front)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "keys": { "type": "string" }, "target": { "type": "string" } },
                "required": ["keys"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        // Enter in a terminal runs whatever is typed there.
        let keys = args.get("keys").and_then(Value::as_str).unwrap_or("").to_lowercase();
        let target = args.get("target").and_then(Value::as_str);
        if (keys.contains("enter") || keys.contains("return")) && into_terminal(target) { Risk::Risky } else { Risk::Normal }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let keys: String = match arg(&args, "keys") {
            Ok(k) => k,
            Err(o) => return o,
        };
        let target: Option<String> = opt_arg(&args, "target");
        let ctx = ctx.clone();
        blocking(move || {
            let into = match focus_target(target.as_deref(), &ctx) {
                Ok(w) => w,
                Err(o) => return o,
            };
            if ctx.dry_run {
                return Outcome::ok(format!("(dry run) would press {keys} in {into}"));
            }
            match platform::press(&keys) {
                Ok(()) => Outcome::ok(format!("pressed {keys} in {into} (its effect isn't checked)")),
                Err(e) => Outcome::failed(format!("{e:#}")),
            }
        })
        .await
    }
}

pub struct TypeText;

#[async_trait]
impl Tool for TypeText {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "type_text".into(),
            description: "Type text into the window in front (or `target`, brought to the front first), where the cursor is. \
                          Click the field first if the cursor isn't in it. Doesn't press enter unless the text ends with a newline."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string" }, "target": { "type": "string" } },
                "required": ["text"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        let text = args.get("text").and_then(Value::as_str).unwrap_or("");
        let target = args.get("target").and_then(Value::as_str);
        if destructive(text) {
            return Risk::NeverSilent;
        }
        // In a terminal, typed text is a command; several lines may send a message anywhere.
        if into_terminal(target) || text.trim_end_matches('\n').contains('\n') { Risk::Risky } else { Risk::Normal }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let text: String = match arg(&args, "text") {
            Ok(t) => t,
            Err(o) => return o,
        };
        let target: Option<String> = opt_arg(&args, "target");
        let ctx = ctx.clone();
        blocking(move || {
            let into = match focus_target(target.as_deref(), &ctx) {
                Ok(w) => w,
                Err(o) => return o,
            };
            if ctx.dry_run {
                return Outcome::ok(format!("(dry run) would type {} characters in {into}", text.chars().count()));
            }
            // In a browser, keys with no text box under the cursor go to the page: typed, but into nothing.
            let in_browser = platform::list_windows().into_iter().find(|w| w.foreground).is_some_and(|w| ["brave", "chrome", "msedge", "firefox", "opera", "vivaldi"].contains(&w.exe.as_str()));
            let nowhere = in_browser && platform::focus_takes_text() == Some(false);
            match platform::type_text(&text) {
                Ok(()) if nowhere => Outcome::unsure(format!("typed {} characters in {into}, but no text box had the cursor, so they went to the page: put it in the box first (click it), or use type_in with the box's name", text.chars().count())),
                Ok(()) => Outcome::ok(format!("typed {} characters in {into}", text.chars().count())),
                Err(e) => Outcome::failed(format!("{e:#}")),
            }
        })
        .await
    }
}

const TERMINALS: &[&str] = &["windowsterminal", "cmd", "powershell", "pwsh", "wsl", "bash", "mintty", "conhost", "openconsole", "alacritty", "wezterm-gui", "wezterm", "kitty", "hyper", "tabby", "warp", "putty", "ubuntu"];

fn is_terminal(w: &WinInfo) -> bool {
    TERMINALS.contains(&w.exe.as_str()) || {
        let t = w.title.to_lowercase();
        ["powershell", "command prompt", "terminal", "bash", "cmd.exe", "wsl"].iter().any(|k| t.contains(k)) && !w.exe.contains("brave") && !w.exe.contains("chrome") && !w.exe.contains("firefox") && !w.exe.contains("msedge")
    }
}

/// Whether input would go to a terminal: the named window, or the one in front.
fn into_terminal(target: Option<&str>) -> bool {
    if let Some(t) = target {
        let t = t.to_lowercase();
        if ["terminal", "powershell", "cmd", "command prompt", "console", "bash", "wsl", "shell"].iter().any(|k| t.contains(k)) {
            return true;
        }
    }
    let wins = platform::list_windows();
    match target.filter(|t| !t.trim().is_empty()) {
        Some(t) => find_window(t, &wins).map(|w| is_terminal(&w)).unwrap_or(false),
        None => wins.iter().find(|w| w.foreground).is_some_and(is_terminal),
    }
}

/// Commands that wipe things. Typing one anywhere always waits for the user, Blackwall or not.
pub fn destructive(text: &str) -> bool {
    let t = format!(" {} ", text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "));
    const BAD: &[&str] = &[
        " rm -rf", " rm -fr", " rm -r ", " rmdir /s", " rd /s", " del /s", " del /q", " erase /s", " format ", " format-volume", " remove-item", " ri -r", " mkfs", " dd if=", " diskpart", " clear-disk", " cipher /w", " vssadmin delete", " bcdedit", " reg delete", " shutdown ", " :(){", " chmod -r 000", " > /dev/sd",
    ];
    BAD.iter().any(|b| t.contains(b))
}

/// Brings the target window forward (if one is named) and says which window input goes to.
fn focus_target(target: Option<&str>, ctx: &Ctx) -> Result<String, Outcome> {
    let wins = others(ctx);
    match target {
        Some(t) if !t.trim().is_empty() => {
            // A field's name where the window's goes ("Message", just clicked into): the window in front, where the
            // cursor is.
            let w = match find_window(t, &wins) {
                Ok(w) => w,
                Err(o) if o.summary.starts_with("no open window matches") => match wins.iter().find(|w| w.foreground) {
                    Some(front) => front.clone(),
                    None => return Err(o),
                },
                Err(o) => return Err(o),
            };
            if !w.foreground && !ctx.dry_run {
                platform::act(w.id, WinAction::Focus).map_err(|e| Outcome::failed(format!("couldn't bring {} to the front: {e:#}", w.app)))?;
            }
            Ok(format!("{} (\"{}\")", w.app, short(&w.title, 60)))
        }
        _ => {
            let fg = platform::list_windows().into_iter().find(|w| w.foreground);
            match fg {
                Some(w) if w.pid == ctx.own_pid => Err(Outcome::failed("her own window is in front; name the window to type into")),
                Some(w) => Ok(format!("{} (\"{}\")", w.app, short(&w.title, 60))),
                None => Ok("the window in front".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(title: &str, app: &str, exe: &str, fg: bool) -> WinInfo {
        WinInfo { id: 1, title: title.into(), app: app.into(), exe: exe.into(), pid: 9, rect: [0; 4], minimized: false, maximized: false, foreground: fg }
    }

    #[test]
    fn finds_by_app_over_title() {
        let wins = vec![win("Spotify – Web Player - Brave", "Brave Browser", "brave", true), win("Spotify Premium", "Spotify", "spotify", false)];
        assert_eq!(find_window("spotify", &wins).unwrap().exe, "spotify");
    }

    #[test]
    fn a_lone_title_match_is_enough() {
        let wins = vec![win("(1) YouTube - Brave", "Brave Browser", "brave", true), win("@friend - Discord", "Discord", "discord", false)];
        assert_eq!(find_window("youtube", &wins).unwrap().exe, "brave");
    }

    #[test]
    fn close_between_two_apps_the_one_in_front() {
        let wins = [win("Gym · volume 49", "Microsoft Edge", "msedge", true), win("Gym - Google Search - Brave", "Brave Browser", "brave", false)];
        assert_eq!(find_window("Gym", &wins).unwrap().exe, "msedge");
        // Neither in front: she asks which.
        let wins = [win("Gym · volume 49", "Microsoft Edge", "msedge", false), win("Gym - Google Search - Brave", "Brave Browser", "brave", false)];
        assert!(find_window("Gym", &wins).is_err());
    }

    #[test]
    fn front_is_front() {
        let wins = vec![win("a", "A", "a", false), win("b", "B", "b", true)];
        assert_eq!(find_window("front", &wins).unwrap().title, "b");
    }

    #[test]
    fn two_windows_of_one_app_is_fine() {
        let wins = vec![win("YouTube - Brave", "Brave Browser", "brave", true), win("GitHub - Brave", "Brave Browser", "brave", false)];
        assert_eq!(find_window("brave", &wins).unwrap().title, "YouTube - Brave");
    }

    #[test]
    fn destructive_commands() {
        assert!(destructive("rm -rf /"));
        assert!(destructive("Remove-Item C:\\ -Recurse -Force"));
        assert!(destructive("format C: /q"));
        assert!(!destructive("ls -la"));
        assert!(!destructive("hello, how's the weather"));
    }

    #[test]
    fn typing_into_a_terminal_is_risky() {
        let t = TypeText;
        assert_eq!(t.risk(&json!({ "text": "rm -rf /", "target": "Terminal" })), Risk::NeverSilent);
        assert_eq!(t.risk(&json!({ "text": "dir", "target": "PowerShell" })), Risk::Risky);
        let k = Keys;
        assert_eq!(k.risk(&json!({ "keys": "enter", "target": "Windows Terminal" })), Risk::Risky);
    }

    #[test]
    fn urls() {
        assert!(looks_like_url("github.com"));
        assert!(looks_like_url("https://x.y/z"));
        assert!(!looks_like_url("steam"));
        assert!(!looks_like_url("v1.2 notes"));
    }

    #[tokio::test]
    async fn search_rejects_unknown_sites() {
        use crate::tool::Status;
        let ctx = Ctx { dry_run: true, ..Ctx::default() };
        let bad = Search.call(json!({ "query": "cats", "site": "stremio" }), &ctx).await;
        assert_eq!(bad.status, Status::Failed, "{}", bad.summary);
        assert!(bad.summary.contains("not \"stremio\""), "{}", bad.summary);
        let ok = Search.call(json!({ "query": "tauri", "site": "github" }), &ctx).await;
        assert_eq!(ok.status, Status::Ok, "{}", ok.summary);
    }

    #[test]
    fn places() {
        assert_eq!(known_place("my downloads").unwrap().0, "Downloads");
        assert_eq!(known_place("Recycle Bin").unwrap().0, "Recycle Bin");
        assert!(known_place("discord").is_none());
    }
}
