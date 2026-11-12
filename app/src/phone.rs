//! Messaging her from your phone: she logs in as your own Discord bot and reads the direct messages you send it, and
//! only yours. Each one is a request like a typed one, shown on her screen but not said (you're not at the PC); her
//! answer comes back in the chat, with "typing…" while she works, and a risky step asks you for a one-time code there
//! first. Off until Settings › Phone has it on, your Discord user ID and the bot's token (kept in Windows' Credential
//! Manager like her API keys). The chat is read every two seconds over Discord's HTTP API: no gateway connection, no
//! extra library, nothing listening on this PC.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use capri_brain::Gate;
use capri_hands::Risk;
use reqwest::Method;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

const API: &str = "https://discord.com/api/v10";
/// Discord wants bots to say who they are this way.
const AGENT: &str = "DiscordBot (https://github.com/theluckytiger/capricorn-engram, 1.0)";
/// Her bot's token, by this name in Credential Manager.
pub const VAULT: &str = "discord";
/// A Discord message holds 2000 characters.
const MAX: usize = 1900;

/// The chat with you, while it's on.
#[derive(Default)]
pub struct Phone {
    /// A risky step waiting for its code: where your next message goes.
    code: Mutex<Option<oneshot::Sender<String>>>,
    /// Working on a message of yours.
    busy: AtomicBool,
    /// That work, for "stop".
    job: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// How it stands, for Settings ("connected as …", "off", what went wrong).
    status: Mutex<String>,
    /// The chat while she's connected, for Settings' hello.
    chat: Mutex<Option<(Discord, String)>>,
}

/// Your messages among `list` (newest first, as Discord gives them), oldest first, and the newest id seen (to ask for
/// what came after it next time). Anyone else's, a bot's (hers), and empty ones are skipped.
fn yours(list: &Value, user: &str, last: u64) -> (Vec<String>, u64) {
    let mut msgs: Vec<(u64, &Value)> = list.as_array().into_iter().flatten().filter_map(|m| Some((m["id"].as_str()?.parse::<u64>().ok()?, m))).filter(|(id, _)| *id > last).collect();
    msgs.sort_by_key(|(id, _)| *id);
    let newest = msgs.last().map(|(id, _)| *id).unwrap_or(last);
    let texts = msgs
        .into_iter()
        .filter(|(_, m)| m["author"]["id"].as_str() == Some(user) && m["author"]["bot"].as_bool() != Some(true))
        .filter_map(|(_, m)| m["content"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(String::from))
        .collect();
    (texts, newest)
}

/// A line cut to what one message holds.
fn fit(line: &str) -> String {
    if line.chars().count() <= MAX { line.to_string() } else { format!("{}…", line.chars().take(MAX).collect::<String>()) }
}

/// A four-digit code for one risky step.
fn new_code() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() ^ (d.as_secs() as u32).rotate_left(13)).unwrap_or(4821);
    format!("{:04}", 1000 + n % 9000)
}

/// "Stop", "cancel", "never mind": whatever she's doing for you, stopped (Esc, from your phone).
fn is_stop(text: &str) -> bool {
    let t: String = text.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect();
    matches!(t.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), "stop" | "stop it" | "stop that" | "cancel" | "cancel it" | "abort" | "never mind" | "nevermind" | "capri stop")
}

/// Whether `reply` is the code (spaces and a trailing full stop don't matter).
fn is_code(reply: &str, code: &str) -> bool {
    reply.trim().trim_end_matches('.').replace(' ', "") == code
}

/// "Screenshot", "show me my screen": a picture of the PC asked for, sent at once (no brain needed, and it works
/// while she's busy with something else: a look at how it's going).
fn wants_screen(text: &str) -> bool {
    let t: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let words: Vec<&str> = t.split_whitespace().filter(|w| !matches!(*w, "capri" | "capricorn" | "please" | "pls" | "hey" | "a" | "me" | "my" | "the" | "can" | "you" | "could")).collect();
    let t = words.join(" ");
    t.contains("screenshot") || t.contains("screen shot") || matches!(t.as_str(), "screen" | "show screen" | "send screen" | "show pc" | "show desktop" | "whats on screen" | "what s on screen")
}

#[derive(Clone)]
struct Discord {
    http: reqwest::Client,
    token: String,
}

impl Discord {
    fn new(token: &str) -> Self {
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(30)).user_agent(AGENT).build().unwrap_or_default();
        Self { http, token: token.trim().to_string() }
    }

    /// One call to Discord's API; told to slow down, it waits as long as it's told (a few times at most).
    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        for _ in 0..4 {
            let mut rb = self.http.request(method.clone(), format!("{}{path}", api())).header("Authorization", format!("Bot {}", self.token));
            if let Some(b) = &body {
                rb = rb.json(b);
            }
            let res = rb.send().await.context("Discord isn't answering")?;
            let status = res.status();
            let v: Value = res.json().await.unwrap_or(Value::Null);
            if status.as_u16() == 429 {
                tokio::time::sleep(Duration::from_secs_f64(v["retry_after"].as_f64().unwrap_or(1.0).clamp(0.5, 30.0))).await;
                continue;
            }
            if !status.is_success() {
                bail!("Discord said {status}{}", v["message"].as_str().map(|m| format!(": {m}")).unwrap_or_default());
            }
            return Ok(v);
        }
        bail!("Discord kept saying to slow down")
    }

    async fn send(&self, channel: &str, line: &str) {
        if let Err(e) = self.call(Method::POST, &format!("/channels/{channel}/messages"), Some(json!({ "content": fit(line) }))).await {
            tracing::warn!("phone: couldn't send: {e:#}");
        }
    }

    /// A message with a picture (a PNG file's bytes), as Discord takes files: one multipart form.
    async fn send_png(&self, channel: &str, line: &str, name: &str, png: Vec<u8>) -> Result<()> {
        let boundary = format!("capri{}", new_code());
        let mut body = Vec::with_capacity(png.len() + 512);
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n").as_bytes());
        body.extend_from_slice(json!({ "content": fit(line) }).to_string().as_bytes());
        body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"{name}\"\r\nContent-Type: image/png\r\n\r\n").as_bytes());
        body.extend_from_slice(&png);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let res = self
            .http
            .post(format!("{}/channels/{channel}/messages", api()))
            .header("Authorization", format!("Bot {}", self.token))
            .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
            .body(body)
            .send()
            .await
            .context("Discord isn't answering")?;
        if !res.status().is_success() {
            bail!("Discord said {}", res.status());
        }
        Ok(())
    }
}

/// Discord's API, or a stand-in for trying her side of the chat without a real bot (`CAPRI_DISCORD_API`, with
/// `CAPRI_DISCORD_TOKEN`).
fn api() -> String {
    std::env::var("CAPRI_DISCORD_API").ok().filter(|a| !a.is_empty()).unwrap_or_else(|| API.to_string())
}

/// Her bot's token, if one is saved.
fn token() -> Option<String> {
    if let Ok(t) = std::env::var("CAPRI_DISCORD_TOKEN")
        && !t.is_empty()
    {
        return Some(t);
    }
    #[cfg(windows)]
    return capri_hands::platform::vault::key_get(VAULT);
    #[cfg(not(windows))]
    None
}

fn wanted(app: &AppHandle) -> Option<String> {
    let phone = app.state::<crate::App>().settings.lock().unwrap().phone.clone();
    let user = phone.discord_user.trim().to_string();
    (phone.on && !user.is_empty()).then_some(user)
}

fn set_status(app: &AppHandle, status: &str) {
    let phone = app.state::<Phone>();
    *phone.status.lock().unwrap() = status.to_string();
    if !status.starts_with("connected") {
        phone.chat.lock().unwrap().take();
    }
}

/// Starts her side of the chat: it waits while it's off, and follows Settings › Phone as it changes.
pub fn start(app: &AppHandle) {
    app.manage(Phone::default());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // After a failure she tries again in 30 s, then waits longer each time up to 5 min (a revoked token
        // shouldn't fill her journal), and at once when Settings › Phone or the token changes.
        let mut wait = 30;
        loop {
            let (Some(user), Some(saved)) = (wanted(&app), token()) else {
                set_status(&app, "off");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            let began = std::time::Instant::now();
            match connect(&app, &Discord::new(&saved), &user).await {
                Ok(()) => wait = 30,
                Err(e) => {
                    tracing::warn!("phone: {e:#}");
                    capri_core::journal::write("phone", serde_json::json!({ "error": format!("{e:#}") }));
                    set_status(&app, &format!("not connected: {e}"));
                    // Connected a good while before it failed: a fresh problem, a short wait.
                    if began.elapsed() > Duration::from_secs(600) {
                        wait = 30;
                    }
                    for _ in 0..wait / 5 {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        if wanted(&app).as_deref() != Some(user.as_str()) || token().as_deref() != Some(saved.as_str()) {
                            break;
                        }
                    }
                    wait = (wait * 2).min(300);
                }
            }
        }
    });
}

/// Connected: reads your messages until Settings › Phone changes (then it's back to `start`'s loop) or Discord fails.
async fn connect(app: &AppHandle, d: &Discord, user: &str) -> Result<()> {
    let me = d.call(Method::GET, "/users/@me", None).await?;
    let bot = me["username"].as_str().unwrap_or("her bot").to_string();
    let dm = d.call(Method::POST, "/users/@me/channels", Some(json!({ "recipient_id": user }))).await.context("couldn't open a chat with you (is the bot in a server you're in?)")?;
    let channel = dm["id"].as_str().context("Discord gave no chat")?.to_string();
    // What's in the chat already isn't news.
    let latest = d.call(Method::GET, &format!("/channels/{channel}/messages?limit=1"), None).await?;
    let mut last = latest[0]["id"].as_str().and_then(|i| i.parse().ok()).unwrap_or(0);
    capri_core::journal::write("phone", serde_json::json!({ "connected": bot }));
    set_status(app, &format!("connected as {bot}"));
    *app.state::<Phone>().chat.lock().unwrap() = Some((d.clone(), channel.clone()));
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        // Turned off, another user, or the token removed or changed in Settings: she lets go at once.
        if wanted(app).as_deref() != Some(user) || token().as_deref() != Some(d.token.as_str()) {
            capri_core::journal::write("phone", serde_json::json!({ "disconnected": "settings changed" }));
            return Ok(());
        }
        let list = d.call(Method::GET, &format!("/channels/{channel}/messages?after={last}&limit=20"), None).await?;
        let (texts, newest) = yours(&list, user, last);
        last = newest;
        for text in texts {
            on_message(app, d, &channel, text).await;
        }
    }
}

/// One message of yours: the code a risky step is waiting for, or a request.
async fn on_message(app: &AppHandle, d: &Discord, channel: &str, text: String) {
    let phone = app.state::<Phone>();
    if let Some(tx) = phone.code.lock().unwrap().take() {
        let _ = tx.send(text);
        return;
    }
    if wants_screen(&text) {
        capri_core::journal::write("phone", serde_json::json!({ "got": text, "sent": "a screenshot" }));
        let (d, channel) = (d.clone(), channel.to_string());
        tauri::async_runtime::spawn(async move {
            let picture = tokio::task::spawn_blocking(screen_png).await.map_err(anyhow::Error::from).and_then(|r| r);
            let sent = match picture {
                Ok(png) => d.send_png(&channel, "Your screen, right now.", "screen.png", png).await,
                Err(e) => Err(e),
            };
            if let Err(e) = sent {
                tracing::warn!("phone: screenshot: {e:#}");
                d.send(&channel, "Couldn't get you a picture of the screen this time.").await;
            }
        });
        return;
    }
    if is_stop(&text) {
        let job = phone.job.lock().unwrap().take();
        let stopped = job.is_some_and(|j| {
            j.abort();
            phone.busy.load(Ordering::Relaxed)
        });
        capri_core::journal::write("phone", serde_json::json!({ "got": text, "stopped": stopped }));
        if stopped {
            // Her screen at the PC stops "thinking" too, as after Esc.
            let _ = tauri::Emitter::emit(app, "capri://answer", serde_json::json!({ "line": "Stopped.", "spoken": false, "steps": [] }));
        }
        d.send(channel, if stopped { "Stopped." } else { "I wasn't doing anything." }).await;
        return;
    }
    if phone.busy.swap(true, Ordering::Relaxed) {
        d.send(channel, "Still on your last one. Send it again when I'm done, or say stop.").await;
        return;
    }
    capri_core::journal::write("phone", serde_json::json!({ "got": text }));
    let (app2, d, channel) = (app.clone(), d.clone(), channel.to_string());
    let job = tauri::async_runtime::spawn(async move {
        let app = app2;
        // "typing…" in the chat while she works (Discord shows it for ten seconds at a time).
        let working = Arc::new(AtomicBool::new(true));
        // Free for your next message and no more "typing…" when this ends, however it ends (stopped too).
        let _free = Free(app.clone(), working.clone());
        {
            let (d, channel, working) = (d.clone(), channel.clone(), working.clone());
            tauri::async_runtime::spawn(async move {
                while working.load(Ordering::Relaxed) {
                    let _ = d.call(Method::POST, &format!("/channels/{channel}/typing"), None).await;
                    tokio::time::sleep(Duration::from_secs(8)).await;
                }
            });
        }
        let gate = ChatGate { app: app.clone(), d: d.clone(), channel: channel.clone() };
        let answer = crate::brain::answer_phone(&app, &text, &gate).await;
        working.store(false, Ordering::Relaxed);
        d.send(&channel, &answer.line).await;
        capri_core::journal::write("phone", serde_json::json!({ "sent": answer.line }));
    });
    *phone.job.lock().unwrap() = Some(job);
}

struct Free(AppHandle, Arc<AtomicBool>);

impl Drop for Free {
    fn drop(&mut self) {
        self.1.store(false, Ordering::Relaxed);
        self.0.state::<Phone>().busy.store(false, Ordering::Relaxed);
    }
}

/// The screen in front as you'd see it (without her on it), as a PNG no bigger than 1920 px a side.
fn screen_png() -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        let shot = capri_hands::platform::screen_shot()?;
        Ok(capri_hands::eyes::png_file(&shot.rgb, shot.width, shot.height, 1920)?.0)
    }
    #[cfg(not(windows))]
    bail!("not on this OS yet")
}

/// Risky steps asked for in the chat: a one-time code, answered within three minutes, or it's a no. Blackwall mode
/// lets the ordinary risky ones through, as at the PC; the never-silent ones always ask.
struct ChatGate {
    app: AppHandle,
    d: Discord,
    channel: String,
}

#[async_trait]
impl Gate for ChatGate {
    async fn allow(&self, what: &str, risk: Risk) -> bool {
        if risk == Risk::Risky && self.app.state::<crate::App>().settings.lock().unwrap().blackwall {
            capri_core::journal::write("confirm", serde_json::json!({ "what": what, "risk": risk, "yes": true, "blackwall": true, "from": "discord" }));
            return true;
        }
        let code = new_code();
        let (tx, rx) = oneshot::channel();
        *self.app.state::<Phone>().code.lock().unwrap() = Some(tx);
        self.d.send(&self.channel, &format!("Do this? {what}\nReply **{code}** to go ahead. Anything else and I won't.")).await;
        let reply = tokio::time::timeout(Duration::from_secs(180), rx).await.ok().and_then(Result::ok);
        self.app.state::<Phone>().code.lock().unwrap().take();
        let yes = reply.as_deref().is_some_and(|r| is_code(r, &code));
        capri_core::journal::write("confirm", serde_json::json!({ "what": what, "risk": risk, "yes": yes, "from": "discord" }));
        yes
    }
}

/// Saves her bot's token once Discord says it works. The answer is for Settings.
#[tauri::command]
pub async fn save_phone_token(token: String) -> Result<String, String> {
    let d = Discord::new(&token);
    let me = d.call(Method::GET, "/users/@me", None).await.map_err(|e| format!("That token didn't work: {e:#}"))?;
    #[cfg(windows)]
    capri_hands::platform::vault::key_set(VAULT, token.trim()).map_err(|e| format!("{e:#}"))?;
    capri_core::journal::write("key", serde_json::json!({ "provider": VAULT, "saved": true }));
    Ok(format!("It works: she's {} on Discord. Saved, encrypted by Windows.", me["username"].as_str().unwrap_or("your bot")))
}

#[tauri::command]
pub fn delete_phone_token() {
    #[cfg(windows)]
    capri_hands::platform::vault::key_delete(VAULT);
    capri_core::journal::write("key", serde_json::json!({ "provider": VAULT, "saved": false }));
}

/// Settings › Phone's "Send me a hello": a first message in your chat, so you see on your phone that she can reach
/// you (a bot that isn't in a server you're in can't).
#[tauri::command]
pub async fn phone_hello(state: tauri::State<'_, Phone>) -> Result<String, String> {
    let Some((d, channel)) = state.chat.lock().unwrap().clone() else { return Err("Not connected yet: turn it on, with your user ID and her token.".into()) };
    d.call(Method::POST, &format!("/channels/{channel}/messages"), Some(json!({ "content": "It's me. Message me here when you're away from the PC." }))).await.map_err(|e| format!("Couldn't message you: {e:#}"))?;
    Ok("Sent. Check Discord.".into())
}

/// For Settings › Phone: whether a token is saved, and how the chat stands.
#[tauri::command]
pub fn phone_status(state: tauri::State<'_, Phone>) -> Value {
    json!({ "token": token().is_some(), "status": state.status.lock().unwrap().clone() })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn only_your_new_messages_oldest_first() {
        let list = json!([
            { "id": "105", "author": { "id": "42" }, "content": "and play some music" },
            { "id": "104", "author": { "id": "7", "bot": true }, "content": "Opening Spotify." },
            { "id": "103", "author": { "id": "99" }, "content": "someone else" },
            { "id": "102", "author": { "id": "42" }, "content": "  open spotify  " },
            { "id": "101", "author": { "id": "42" }, "content": "" },
            { "id": "100", "author": { "id": "42" }, "content": "seen already" }
        ]);
        let (texts, newest) = super::yours(&list, "42", 100);
        assert_eq!(texts, ["open spotify", "and play some music"]);
        assert_eq!(newest, 105);
        // Nothing new: where it was.
        assert_eq!(super::yours(&json!([]), "42", 105), (Vec::<String>::new(), 105));
    }

    #[test]
    fn the_code_and_nothing_else() {
        assert!(super::is_code("4821", "4821"));
        assert!(super::is_code(" 48 21. ", "4821"));
        assert!(!super::is_code("yes", "4821"));
        assert!(!super::is_code("4822", "4821"));
        let c = super::new_code();
        assert!(c.len() == 4 && c.chars().all(|d| d.is_ascii_digit()) && !c.starts_with('0'), "{c}");
    }

    #[test]
    fn a_screenshot_asked_for() {
        for t in ["screenshot", "Screenshot pls", "send me a screenshot", "Capri, show me my screen", "show me the screen", "what's on my screen?", "screen"] {
            assert!(super::wants_screen(t), "{t}");
        }
        for t in ["close the screen saver settings", "what time is it", "show me my emails", "is my download done", "take a screenshot of discord and send it to Ana"] {
            assert!(!super::wants_screen(t) || t.contains("screenshot"), "{t}");
        }
    }

    #[test]
    fn stop_means_stop() {
        for t in ["stop", "Stop!", "STOP IT", "cancel", "never mind", "Capri, stop"] {
            assert!(super::is_stop(t), "{t}");
        }
        for t in ["stop the music", "don't stop", "cancel my reminder", "stopwatch"] {
            assert!(!super::is_stop(t), "{t}");
        }
    }

    #[test]
    fn long_lines_fit_a_message() {
        assert_eq!(super::fit("short"), "short");
        assert_eq!(super::fit(&"x".repeat(5000)).chars().count(), super::MAX + 1);
    }
}
