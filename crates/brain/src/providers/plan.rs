//! Her brain on your own plan: the command-line agent that comes with it (Claude Code for a Claude Pro or Max plan,
//! Cursor's agent for a Cursor plan), on this PC, signed in with your account in that app, never by her. Each turn is
//! one run of it, in an empty folder of hers, with none of its own tools, plugins, hooks or MCP servers and nothing
//! saved: it only thinks. Her tools go in as text, and its tool calls come back as text she reads. Your plan pays, not
//! an API key (one in the environment is taken out of its way, so it isn't billed by mistake).

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::llm::{Call, Effort, Model, Msg, Reply, Req, Role, Stop, Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Claude,
    Cursor,
    Codex,
}

pub struct PlanBrain {
    id: String,
    kind: Kind,
    model: String,
    exe: PathBuf,
    /// Claude Code kept open for the job at hand: each step sends only what's new (the tool results), not the whole
    /// conversation again, and doesn't wait for the app to start (about 3 s a step).
    session: tokio::sync::Mutex<Option<Session>>,
    /// Cursor's chat for the job at hand: after the first step it's told only what's new.
    cursor_chat: tokio::sync::Mutex<Option<CursorChat>>,
}

/// A chat of Cursor's agent, open for the steps of one job. Each step is still a fresh run of the app, but it reads about
/// 150 new tokens instead of her whole conversation again.
struct CursorChat {
    id: String,
    /// Her instructions it was started with.
    system: u64,
    /// The messages it has seen, as fingerprints.
    seen: Vec<u64>,
    at: Instant,
}

/// Cursor's agent as a server (its ACP mode), started once and kept: each step is one prompt to it. Started fresh for
/// every step it spent 5 to 10 s getting going before it read a word (steps took 10 to 45 s; this way 3 to 4), and a
/// picture had to be saved for it to open with a tool of its own (a minute, and the job timed out). Here pictures go
/// in with the words.
struct Acp {
    _child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    next: u64,
    /// Its models as it names them ("composer-2.5[fast=true]").
    models: Vec<String>,
    /// A session made ahead: its first one takes 11 to 17 s, later ones 1.5 s.
    spare: Option<String>,
}

fn acp() -> &'static tokio::sync::Mutex<Option<Acp>> {
    static ACP: std::sync::OnceLock<tokio::sync::Mutex<Option<Acp>>> = std::sync::OnceLock::new();
    ACP.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// Set when this Cursor has no server mode (an old version): its one-run-a-step way is used instead.
static NO_ACP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const NO_SERVER: &str = "no server mode";

/// Node and the script behind `cursor-agent.cmd`, run straight: the .cmd goes through cmd and PowerShell first, and
/// stopping cmd left node running.
fn cursor_node(exe: &Path) -> Option<(PathBuf, PathBuf)> {
    let versions = exe.parent()?.join("versions");
    let key = |n: &str| -> Option<(u32, u32, u32)> {
        let mut p = n.split('-').next()?.split('.').map(|x| x.parse::<u32>().ok());
        Some((p.next()??, p.next()??, p.next()??))
    };
    let newest = std::fs::read_dir(&versions)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| key(&e.file_name().to_string_lossy()).map(|k| (k, e.path())))
        .max_by_key(|(k, _)| *k)?
        .1;
    let (node, script) = (newest.join("node.exe"), newest.join("index.js"));
    (node.is_file() && script.is_file()).then_some((node, script))
}

/// Her name for a Cursor model as its server names it: "composer-2.5-fast" is "composer-2.5[fast=true]", "auto" is
/// "default[]".
fn acp_model(model: &str, listed: &[String]) -> Option<String> {
    if model == "auto" {
        return listed.iter().find(|m| m.starts_with("default")).cloned();
    }
    if listed.iter().any(|m| m == model) {
        return Some(model.to_string());
    }
    let fast = model.ends_with("-fast");
    let base = model.trim_end_matches("-fast");
    let same: Vec<&String> = listed.iter().filter(|m| m.split('[').next() == Some(base)).collect();
    same.iter().find(|m| m.contains("fast=true") == fast).or(same.first()).map(|m| m.to_string())
}

impl Acp {
    async fn start(exe: &Path, dir: &Path) -> Result<Acp> {
        let mut cmd = match cursor_node(exe) {
            Some((node, script)) => {
                let mut c = command(&node, &[script.to_string_lossy().into_owned(), "acp".into()], dir);
                if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                    c.env("NODE_COMPILE_CACHE", PathBuf::from(local).join("cursor-compile-cache"));
                }
                c.env("CURSOR_INVOKED_AS", "cursor-agent.cmd");
                c
            }
            None => command(exe, &["acp".to_string()], dir),
        };
        if let Some(home) = cursor_home() {
            cmd.env("USERPROFILE", &home).env("HOME", &home);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().with_context(|| format!("couldn't start {}", exe.display()))?;
        // It stays running when she quits (its input closing isn't enough): it goes with her.
        #[cfg(windows)]
        if let Some(pid) = child.id() {
            let _ = capri_hands::platform::end_with_me(pid);
        }
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let mut acp = Acp { _child: child, stdin, lines: BufReader::new(stdout).lines(), next: 0, models: Vec::new(), spare: None };
        let hello = json!({ "protocolVersion": 1, "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false } });
        acp.call("initialize", hello, Duration::from_secs(30)).await.map_err(|e| anyhow!("{NO_SERVER}: {e:#}"))?;
        Ok(acp)
    }

    /// One request, and what it said (its message chunks) while answering. Its own requests (permission for a tool of
    /// its own, reading a file) are turned down: it only thinks.
    async fn call(&mut self, method: &str, params: Value, wait: Duration) -> Result<(Value, String)> {
        self.next += 1;
        let id = self.next;
        let line = format!("{}\n", json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let mut said = String::new();
        let (stdin, lines) = (&mut self.stdin, &mut self.lines);
        let answer = tokio::time::timeout(wait, async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
            while let Some(l) = lines.next_line().await? {
                let Ok(m) = serde_json::from_str::<Value>(&l) else { continue };
                if m["id"].as_u64() == Some(id) && m.get("method").is_none() {
                    if let Some(e) = m.get("error") {
                        bail!("{}", e["data"]["message"].as_str().or(e["message"].as_str()).unwrap_or("an error"));
                    }
                    return Ok(m["result"].clone());
                }
                if m["method"] == "session/update" {
                    let u = &m["params"]["update"];
                    if u["sessionUpdate"] == "agent_message_chunk" {
                        said.push_str(u["content"]["text"].as_str().unwrap_or(""));
                    }
                } else if let (Some(asked), Some(rid)) = (m["method"].as_str(), m.get("id")) {
                    let reply = if asked == "session/request_permission" {
                        json!({ "jsonrpc": "2.0", "id": rid, "result": { "outcome": { "outcome": "cancelled" } } })
                    } else {
                        json!({ "jsonrpc": "2.0", "id": rid, "error": { "code": -32601, "message": "not available here" } })
                    };
                    stdin.write_all(format!("{reply}\n").as_bytes()).await?;
                    stdin.flush().await?;
                }
            }
            bail!("{} stopped", app(Kind::Cursor))
        })
        .await;
        let result = answer.map_err(|_| anyhow!("no answer in {} s", wait.as_secs()))??;
        Ok((result, said))
    }

    async fn new_session(&mut self, dir: &Path) -> Result<String> {
        let (r, _) = self.call("session/new", json!({ "cwd": dir, "mcpServers": [] }), Duration::from_secs(60)).await?;
        if self.models.is_empty() {
            self.models = r["models"]["availableModels"].as_array().into_iter().flatten().filter_map(|m| m["modelId"].as_str().map(String::from)).collect();
        }
        r["sessionId"].as_str().map(String::from).ok_or_else(|| anyhow!("{} made no session", app(Kind::Cursor)))
    }

    /// A session for one job (or one line of hers): read-only, on the model she was given.
    async fn session(&mut self, dir: &Path, model: &str) -> Result<String> {
        let sid = match self.spare.take() {
            Some(s) => s,
            None => self.new_session(dir).await?,
        };
        let _ = self.call("session/set_mode", json!({ "sessionId": sid, "modeId": "ask" }), Duration::from_secs(15)).await;
        if let Some(m) = acp_model(model, &self.models) {
            self.call("session/set_model", json!({ "sessionId": sid, "modelId": m }), Duration::from_secs(15)).await?;
        }
        Ok(sid)
    }

    async fn cancel(&mut self, sid: &str) {
        let note = format!("{}\n", json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": sid } }));
        let _ = self.stdin.write_all(note.as_bytes()).await;
        let _ = self.stdin.flush().await;
    }
}

/// Cursor's server started ahead, with a session ready, so her first job on it waits for neither.
async fn warm_cursor(exe: &Path) {
    let Ok(dir) = folder() else { return };
    let mut slot = acp().lock().await;
    if slot.is_none() {
        match Acp::start(exe, &dir).await {
            Ok(a) => *slot = Some(a),
            Err(e) => {
                if e.to_string().starts_with(NO_SERVER) {
                    NO_ACP.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                tracing::warn!("Cursor's server didn't start: {e:#}");
                return;
            }
        }
    }
    if let Some(server) = slot.as_mut()
        && server.spare.is_none()
        && let Ok(sid) = server.new_session(&dir).await
    {
        server.spare = Some(sid);
    }
}

/// One Claude Code run, open for the turns of one job.
struct Session {
    _child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    /// Her instructions and effort it was started with.
    system: u64,
    /// The messages it has seen, as fingerprints.
    seen: Vec<u64>,
    at: Instant,
}

fn fingerprint(m: &Msg) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (m.role as u8).hash(&mut h);
    m.text.hash(&mut h);
    for c in &m.calls {
        c.name.hash(&mut h);
        c.args.to_string().hash(&mut h);
    }
    m.call_id.hash(&mut h);
    m.images.len().hash(&mut h);
    h.finish()
}

/// How long one turn may take before it's given up on (her local brain takes over). The slowest step ever logged took 35 s;
/// at 150 s she sat there saying "Got it" to everything you said for two and a half minutes.
const TURN: Duration = Duration::from_secs(60);

impl PlanBrain {
    /// `claudecode/sonnet`, `cursor/auto`: the app must be installed (signed in is checked on the first turn).
    pub fn new(id: &str, kind: Kind, model: &str) -> Result<Self> {
        let exe = find(kind).ok_or_else(|| anyhow!("{} isn't installed on this PC", app(kind)))?;
        Ok(Self { id: id.into(), kind, model: model.into(), exe, session: tokio::sync::Mutex::new(None), cursor_chat: tokio::sync::Mutex::new(None) })
    }
}

fn app(kind: Kind) -> &'static str {
    match kind {
        Kind::Claude => "Claude Code",
        Kind::Cursor => "Cursor's agent",
        Kind::Codex => "Codex",
    }
}

/// Where the app's installer puts it: its own folder, npm's, or anywhere on PATH.
pub fn find(kind: Kind) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
    let (spots, names): (Vec<PathBuf>, &[&str]) = match kind {
        Kind::Claude => (vec![home.join(".local/bin/claude.exe"), roaming.join("npm/claude.cmd")], &["claude.exe", "claude.cmd"]),
        Kind::Cursor => (vec![local.join("cursor-agent/cursor-agent.cmd"), home.join(".local/bin/cursor-agent.exe")], &["cursor-agent.cmd", "cursor-agent.exe", "cursor-agent"]),
        Kind::Codex => (vec![roaming.join("npm/codex.cmd"), home.join(".local/bin/codex.exe")], &["codex.exe", "codex.cmd"]),
    };
    spots.into_iter().find(|p| p.is_file()).or_else(|| {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
    })
}

/// Her folder for its runs: empty, so it reads none of your projects or settings.
fn folder() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("capri-plan-brain");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// How a brain without tools of its own is told about hers, and how to call one.
fn protocol(tools: &[capri_hands::ToolSpec]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let list: Vec<String> = tools.iter().map(|t| format!("- {}: {}\n  arguments (JSON schema): {}", t.name, t.description, t.parameters)).collect();
    format!(
        "\n\n# Your tools\nYou act on the user's PC only through these tools (you have no other tools, files or commands). To \
         use one, write:\n<invoke name=\"tool_name\">\n<parameter name=\"argument\">value</parameter>\n</invoke>\n(an \
         argument that's a number, true/false, a list or an object is written as JSON). You may write several in a row; \
         then stop, and their results come back to you. When the job is done, or no tool is needed, answer in plain text \
         with no invoke.\n\n{}",
        list.join("\n")
    )
}

/// What a coding agent is told first when it's running as her brain. Cursor's and Codex's agents have tools of their
/// own and take hers for some of theirs that have gone missing: left alone, Composer looked for `click` in its own
/// list, found nothing and answered "I'm in Ask mode, I can't run that" on 4 of 16 jobs.
const AGENT_FRAME: &str = "You are being run by a program, not by someone at an editor. The program is Capri, a desktop assistant on \
this user's PC, and your answer is only text that she reads.\n\nThe tools under \"Your tools\" below are hers. They aren't \
in your own tool list and never will be, so don't look for them there and don't try to call them that way. They run like \
this: you write an <invoke> block as plain text in your answer, Capri reads it, runs that tool on the PC herself, and her \
next message to you has the result. Writing the block is all it takes, whatever mode you're in, and it really does \
happen.\n\nSo use none of your own tools (no searching, no shell, no reading files except a picture you're told to open), don't mention this program, your editor or \
what mode you're in, and don't give the user steps to do themselves. Every answer is either the <invoke> block(s) for the \
next step or, when the job is done or nothing needs doing, the plain answer.";

/// And last, right where it answers.
const AGENT_LAST: &str = "(Now write your next step: the <invoke> block(s) if there's anything left to do, otherwise the plain answer. A block you write is run for you.)";

/// Telling a coding agent to open the pictures that came with the last result.
fn opening(pictures: &[String]) -> String {
    if pictures.is_empty() {
        return String::new();
    }
    format!("The picture that came with the last result is in this folder: {}. Open it with your file-reading tool, the one tool of yours to use here, and look at it before you answer.\n\n", pictures.join(", "))
}

/// Her instructions and the conversation as the one prompt a coding agent gets. `pictures`: what came with the last
/// result, as files in its folder.
fn agent_prompt(system: &str, prompt: &str, pictures: &[String]) -> String {
    let look = opening(pictures);
    if system.contains("# Your tools") {
        format!("{AGENT_FRAME}\n\n{system}\n\n# The conversation so far\n{prompt}\n\n{look}{AGENT_LAST}")
    } else {
        format!("{system}\n\n# The conversation so far\n{prompt}{}", if look.is_empty() { String::new() } else { format!("\n\n{}", look.trim_end()) })
    }
}

/// A later step of a job, in a chat that already has her instructions.
fn agent_step(prompt: &str, pictures: &[String]) -> String {
    format!("{}{prompt}\n\n{AGENT_LAST}", opening(pictures))
}

/// The pictures that came with the latest results, saved in `dir` for an agent that opens files (each step is a fresh
/// run and what it's handed is text). Their file names.
fn save_pictures(msgs: &[Msg], dir: &Path) -> Vec<String> {
    use base64::Engine;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut names = Vec::new();
    for image in msgs.iter().rev().take_while(|m| m.role != Role::Assistant).flat_map(|m| &m.images) {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&image.base64) else { continue };
        let name = format!("picture-{stamp}-{}.{}", names.len(), image.mime.rsplit('/').next().unwrap_or("png"));
        if std::fs::write(dir.join(&name), bytes).is_ok() {
            names.push(name);
        }
    }
    names
}

/// A home for Cursor's agent with its own folder in and Claude's not. It imports the hooks in `~/.claude/settings.json`
/// and runs each on every step: a few installed hooks cost 4.5 s of an 11 s step.
fn cursor_home() -> Option<PathBuf> {
    let real = directories::BaseDirs::new()?.home_dir().join(".cursor");
    let home = folder().ok()?.join("home");
    let link = home.join(".cursor");
    if !link.exists() {
        std::fs::create_dir_all(&home).ok()?;
        // A junction: unlike a symbolic link, it needs no special rights on Windows.
        let mut mklink = std::process::Command::new("cmd.exe");
        mklink.args(["/d", "/c", "mklink", "/J"]).arg(&link).arg(&real).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut mklink, 0x0800_0000);
        mklink.status().ok()?;
    }
    link.exists().then_some(home)
}

/// A failure that's worth one more try: the network dropped, not a sign-in or a used-up plan.
fn passing(error: &str) -> bool {
    let low = error.to_lowercase();
    ["econnreset", "etimedout", "econnrefused", "socket hang up", "network", "connection", "fetch failed", "temporarily", "try again"].iter().any(|w| low.contains(w))
}

/// The conversation as one prompt (each turn is a fresh run): oldest first, her tool calls and their results as
/// they happened.
fn transcript(msgs: &[Msg]) -> String {
    let mut out = String::new();
    for m in msgs {
        match m.role {
            Role::User => out.push_str(&format!("[User]\n{}\n\n", m.text)),
            Role::Assistant => {
                out.push_str("[You]\n");
                if !m.text.trim().is_empty() {
                    out.push_str(m.text.trim());
                    out.push('\n');
                }
                for c in &m.calls {
                    out.push_str(&format!("<tool_call>{}</tool_call>\n", json!({ "name": c.name, "arguments": c.args })));
                }
                out.push('\n');
            }
            Role::Tool => out.push_str(&format!("[Tool result: {}{}]\n{}\n\n", m.tool_name.as_deref().unwrap_or("tool"), if m.is_error { ", failed" } else { "" }, m.text)),
        }
    }
    if msgs.last().is_some_and(|m| m.role == Role::Tool) {
        out.push_str("[Your next step]\n");
    }
    out
}

/// Its reply: the words, and the tool calls written in it, as `<invoke name="…"><parameter name="…">…</parameter>
/// </invoke>` (how Claude writes them by habit, inside `<function_calls>` or not) or `<tool_call>{json}</tool_call>`.
pub fn parse(text: &str) -> (String, Vec<Call>) {
    let (text, mut calls) = invokes(text);
    let (words, more) = tool_calls(&text);
    calls.extend(more);
    for (i, c) in calls.iter_mut().enumerate() {
        c.id = format!("call_{i}");
    }
    (words, calls)
}

/// `<invoke name="open"><parameter name="name">Discord</parameter></invoke>` calls, and the text without them.
fn invokes(text: &str) -> (String, Vec<Call>) {
    let mut calls = Vec::new();
    let mut words = String::new();
    let mut rest = text;
    while let Some(a) = rest.find("<invoke name=\"") {
        words.push_str(&rest[..a]);
        let after = &rest[a + "<invoke name=\"".len()..];
        let Some(q) = after.find('"') else { break };
        let name = after[..q].to_string();
        let (body, next) = match after.find("</invoke>") {
            Some(b) => (&after[q..b], &after[b + "</invoke>".len()..]),
            None => (&after[q..], ""),
        };
        let mut args = serde_json::Map::new();
        let mut p = body;
        while let Some(s) = p.find("<parameter name=\"") {
            let after = &p[s + "<parameter name=\"".len()..];
            let Some(q) = after.find('"') else { break };
            let key = after[..q].to_string();
            let Some(gt) = after[q..].find('>') else { break };
            let value_start = &after[q + gt + 1..];
            let Some(end) = value_start.find("</parameter>") else { break };
            let raw = &value_start[..end];
            // A number, true/false, a list or an object as JSON, and text with its quotes on is still that text (Cursor's
            // Auto wrote "bottom", and the page scrolled nowhere); anything else is the text as written.
            let value = serde_json::from_str::<Value>(raw.trim()).unwrap_or_else(|_| Value::String(raw.to_string()));
            args.insert(key, value);
            p = &value_start[end + "</parameter>".len()..];
        }
        // Shown `<parameter name="argument">` as the example, Composer Fast once wrote all of them as JSON in that one.
        if args.len() == 1
            && let Some(Value::Object(inner)) = ["argument", "arguments", "args"].iter().find_map(|k| args.get(*k))
        {
            args = inner.clone();
        }
        calls.push(Call { id: String::new(), name, args: Value::Object(args), extra: Value::Null });
        rest = next;
    }
    words.push_str(rest);
    let words = words.replace("<function_calls>", "").replace("</function_calls>", "");
    (words, calls)
}

/// `<tool_call>{"name": …, "arguments": …}</tool_call>` calls, and the text without them.
fn tool_calls(text: &str) -> (String, Vec<Call>) {
    let mut calls = Vec::new();
    let mut words = String::new();
    let mut rest = text;
    while let Some(a) = rest.find("<tool_call>") {
        words.push_str(&rest[..a]);
        let after = &rest[a + "<tool_call>".len()..];
        let (body, next) = match after.find("</tool_call>") {
            Some(b) => (&after[..b], &after[b + "</tool_call>".len()..]),
            None => (after, ""),
        };
        let body = body.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
        if let Ok(v) = serde_json::from_str::<Value>(body) {
            let name = v["name"].as_str().unwrap_or_default().to_string();
            let args = if v["arguments"].is_object() { v["arguments"].clone() } else if v["args"].is_object() { v["args"].clone() } else { json!({}) };
            if !name.is_empty() {
                calls.push(Call { id: format!("call_{}", calls.len()), name, args, extra: Value::Null });
            }
        }
        rest = next;
    }
    words.push_str(rest);
    (words.trim().to_string(), calls)
}

#[async_trait]
impl Model for PlanBrain {
    fn id(&self) -> &str {
        &self.id
    }

    fn local(&self) -> bool {
        false
    }

    fn sees(&self) -> bool {
        matches!(self.kind, Kind::Claude | Kind::Cursor)
    }

    fn sees_slowly(&self) -> bool {
        self.kind == Kind::Cursor
    }

    async fn warm(&self) {
        if self.kind == Kind::Cursor && !NO_ACP.load(std::sync::atomic::Ordering::Relaxed) {
            warm_cursor(&self.exe).await;
        }
    }

    async fn chat(&self, req: Req<'_>) -> Result<Reply> {
        let dir = folder()?;
        let system = format!("{}{}", req.system, protocol(req.tools));
        let prompt = transcript(req.msgs);
        let (text, usage) = match self.kind {
            Kind::Claude => self.claude(&dir, &system, req.msgs, req.effort).await?,
            Kind::Cursor if !NO_ACP.load(std::sync::atomic::Ordering::Relaxed) => match self.cursor_server(&dir, &system, req.msgs).await {
                Err(e) if e.to_string().starts_with(NO_SERVER) => {
                    NO_ACP.store(true, std::sync::atomic::Ordering::Relaxed);
                    capri_core::journal::write("brain", json!({ "model": self.id, "cursor": "no server mode; one run a step" }));
                    self.cursor_runs(&dir, &system, req.msgs).await?
                }
                other => other?,
            },
            Kind::Cursor => self.cursor_runs(&dir, &system, req.msgs).await?,
            Kind::Codex => (self.codex(&dir, &system, &prompt).await?, Usage::default()),
        };
        let (words, calls) = parse(&text);
        // These agents write for a screen that shows Markdown; her panel and voice don't.
        let words = words.replace("**", "");
        let stop = if calls.is_empty() { Stop::Done } else { Stop::ToolUse };
        Ok(Reply { text: words, calls, raw: None, stop, usage })
    }
}

impl PlanBrain {
    async fn claude(&self, dir: &Path, system: &str, msgs: &[Msg], effort: Effort) -> Result<(String, Usage)> {
        let effort = match effort {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        };
        let key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (system, effort).hash(&mut h);
            h.finish()
        };
        let prints: Vec<u64> = msgs.iter().map(fingerprint).collect();
        let mut guard = self.session.lock().await;
        // The same job going on (the conversation so far is what it has seen, and more): the session it's in. Else a
        // new one (the old one goes with it).
        let going_on = guard.as_ref().is_some_and(|s| s.system == key && s.at.elapsed() < Duration::from_secs(600) && prints.len() > s.seen.len() && prints.starts_with(&s.seen));
        if !going_on {
            *guard = None;
            *guard = Some(self.start_claude(dir, system, effort).await?);
        }
        let session = guard.as_mut().expect("a session");
        let new = &msgs[session.seen.len()..];
        // Its own replies it has already: only what came after them (the tool results), or the whole conversation
        // the first time.
        let prompt = if session.seen.is_empty() { transcript(new) } else { transcript(&new.iter().filter(|m| m.role != Role::Assistant).cloned().collect::<Vec<_>>()) };
        let mut content = vec![json!({ "type": "text", "text": prompt })];
        for i in new.iter().filter(|m| m.role != Role::Assistant).flat_map(|m| &m.images) {
            content.push(json!({ "type": "image", "source": { "type": "base64", "media_type": i.mime, "data": i.base64 } }));
        }
        let line = format!("{}\n", json!({ "type": "user", "message": { "role": "user", "content": content } }));
        // What it last said while it worked, for the log when it never answers (it retries quietly when the service is busy).
        let mut last_said = String::new();
        let answer = tokio::time::timeout(TURN, async {
            session.stdin.write_all(line.as_bytes()).await?;
            session.stdin.flush().await?;
            while let Some(l) = session.lines.next_line().await? {
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    if v["type"] == "result" {
                        return Ok::<Value, anyhow::Error>(v);
                    }
                    last_said = format!("{} {}", v["type"].as_str().unwrap_or(""), v["subtype"].as_str().unwrap_or(""));
                    if v["subtype"].as_str().is_some_and(|s| s.contains("retry")) {
                        capri_core::journal::write("brain waiting", json!({ "model": self.id, "said": short(&l) }));
                    }
                }
            }
            Err(anyhow!("{} stopped without an answer", app(self.kind)))
        })
        .await;
        let r = match answer {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                *guard = None;
                return Err(e);
            }
            Err(_) => {
                *guard = None;
                bail!("no answer in {} s (the last thing it did: {})", TURN.as_secs(), if last_said.trim().is_empty() { "nothing" } else { last_said.trim() });
            }
        };
        let text = r["result"].as_str().unwrap_or_default().to_string();
        if r["is_error"].as_bool() == Some(true) || r["subtype"].as_str().is_some_and(|s| s != "success") {
            *guard = None;
            bail!("{}", explain(Kind::Claude, &text));
        }
        session.seen = prints;
        session.at = Instant::now();
        let u = &r["usage"];
        Ok((text, Usage { input: u["input_tokens"].as_u64().unwrap_or(0), output: u["output_tokens"].as_u64().unwrap_or(0), cached: u["cache_read_input_tokens"].as_u64().unwrap_or(0) }))
    }

    /// Claude Code started hidden in her folder, reading turns as JSON lines and writing its events as JSON lines.
    async fn start_claude(&self, dir: &Path, system: &str, effort: &str) -> Result<Session> {
        let sys_file = dir.join(format!("system-{}.md", std::process::id()));
        std::fs::write(&sys_file, system)?;
        let args: Vec<String> = [
            "-p", "--verbose", "--input-format", "stream-json", "--output-format", "stream-json", "--tools", "", "--strict-mcp-config", "--no-session-persistence", "--safe-mode", "--disable-slash-commands",
            "--setting-sources", "project", "--model", &self.model, "--effort", effort, "--system-prompt-file",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([sys_file.to_string_lossy().into_owned()])
        .collect();
        let mut cmd = command(&self.exe, &args, dir);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().with_context(|| format!("couldn't start {}", self.exe.display()))?;
        #[cfg(windows)]
        if let Some(pid) = child.id() {
            let _ = capri_hands::platform::end_with_me(pid);
        }
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        Ok(Session { _child: child, stdin, lines: BufReader::new(stdout).lines(), system: 0, seen: Vec::new(), at: Instant::now() })
            .map(|mut s| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (system, effort).hash(&mut h);
                s.system = h.finish();
                s
            })
    }

    /// Cursor's server: a job's steps go on in one session, each a prompt with only what's new, pictures in it.
    async fn cursor_server(&self, dir: &Path, system: &str, msgs: &[Msg]) -> Result<(String, Usage)> {
        let key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            system.hash(&mut h);
            h.finish()
        };
        let prints: Vec<u64> = msgs.iter().map(fingerprint).collect();
        let job = system.contains("# Your tools");
        let mut slot = acp().lock().await;
        if slot.is_none() {
            *slot = Some(Acp::start(&self.exe, dir).await?);
        }
        let server = slot.as_mut().expect("started");
        let mut chat = self.cursor_chat.lock().await;
        let going_on = chat.as_ref().filter(|c| job && c.system == key && c.at.elapsed() < Duration::from_secs(600) && prints.len() > c.seen.len() && prints.starts_with(&c.seen)).map(|c| (c.id.clone(), c.seen.len()));
        let started = Instant::now();
        let (sid, text) = match going_on {
            Some((id, seen)) => {
                let new: Vec<Msg> = msgs[seen..].iter().filter(|m| m.role != Role::Assistant).cloned().collect();
                (id, agent_step(&transcript(&new), &[]))
            }
            None => match server.session(dir, &self.model).await {
                Ok(sid) => (sid, agent_prompt(system, &transcript(msgs), &[])),
                Err(e) => {
                    *slot = None;
                    return Err(e);
                }
            },
        };
        let mut prompt = vec![json!({ "type": "text", "text": text })];
        for i in msgs.iter().rev().take_while(|m| m.role != Role::Assistant).flat_map(|m| &m.images) {
            prompt.push(json!({ "type": "image", "mimeType": i.mime, "data": i.base64 }));
        }
        // Its own trouble comes back as what it "said" ("Error: RetriableError: Connection stalled"): taken as its
        // answer, that ended a job. It's an error, so the step is asked again.
        let answer = server.call("session/prompt", json!({ "sessionId": sid, "prompt": prompt }), TURN).await.and_then(|(_, said)| match said.trim().strip_prefix("Error:") {
            Some(trouble) if said.trim().len() < 400 => Err(anyhow!("{} had trouble:{trouble}", app(Kind::Cursor))),
            _ => Ok(said),
        });
        match answer {
            Ok(said) => {
                *chat = job.then(|| CursorChat { id: sid, system: key, seen: prints, at: Instant::now() });
                tracing::debug!("cursor server step: {} ms", started.elapsed().as_millis());
                Ok((said, Usage::default()))
            }
            Err(e) => {
                *chat = None;
                if e.to_string().contains("stopped") || e.to_string().to_lowercase().contains("pipe") {
                    *slot = None;
                } else {
                    server.cancel(&sid).await;
                }
                Err(e)
            }
        }
    }

    /// Cursor's way without its server: one run of the app a step, pictures saved for it to open.
    async fn cursor_runs(&self, dir: &Path, system: &str, msgs: &[Msg]) -> Result<(String, Usage)> {
        let pictures = save_pictures(msgs, dir);
        let answer = self.cursor(dir, system, msgs, &pictures).await;
        for name in &pictures {
            let _ = std::fs::remove_file(dir.join(name));
        }
        answer
    }

    async fn cursor(&self, dir: &Path, system: &str, msgs: &[Msg], pictures: &[String]) -> Result<(String, Usage)> {
        let key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            system.hash(&mut h);
            h.finish()
        };
        let prints: Vec<u64> = msgs.iter().map(fingerprint).collect();
        // A job's steps carry on in one chat. Anything else (a line in her voice, a look) is its own.
        let job = system.contains("# Your tools");
        let mut chat = self.cursor_chat.lock().await;
        let going_on = chat.as_ref().filter(|c| job && c.system == key && c.at.elapsed() < Duration::from_secs(600) && prints.len() > c.seen.len() && prints.starts_with(&c.seen));
        let mut args: Vec<String> = ["-p", "--output-format", "json", "--mode", "ask", "--trust", "--model", &self.model].iter().map(|s| s.to_string()).collect();
        // Its agent takes no system prompt of its own, so hers goes in the one prompt it gets, the first time.
        let whole = match going_on {
            Some(c) => {
                args.extend(["--resume".to_string(), c.id.clone()]);
                // Its own replies it has already: only what came after them (the tool results).
                let new: Vec<Msg> = msgs[c.seen.len()..].iter().filter(|m| m.role != Role::Assistant).cloned().collect();
                agent_step(&transcript(&new), pictures)
            }
            None => agent_prompt(system, &transcript(msgs), pictures),
        };
        let out = match run(Kind::Cursor, &self.exe, &args, dir, Some(whole.clone())).await {
            Err(e) if passing(&e.to_string()) => run(Kind::Cursor, &self.exe, &args, dir, Some(whole)).await?,
            other => other?,
        };
        let v: Value = out.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok()).ok_or_else(|| anyhow!("{} gave no answer: {}", app(self.kind), short(&out)))?;
        let text = v["result"].as_str().or(v["text"].as_str()).unwrap_or_default().to_string();
        if v["is_error"].as_bool() == Some(true) {
            bail!("{}", explain(Kind::Cursor, &text));
        }
        if job {
            *chat = v["session_id"].as_str().map(|id| CursorChat { id: id.to_string(), system: key, seen: prints, at: Instant::now() });
        }
        let u = &v["usage"];
        Ok((text, Usage { input: u["inputTokens"].as_u64().unwrap_or(0), output: u["outputTokens"].as_u64().unwrap_or(0), cached: u["cacheReadTokens"].as_u64().unwrap_or(0) }))
    }
}

impl PlanBrain {
    /// Codex: one prompt on stdin, its reply written to a file, none of its own tools. (Built from its documented
    /// flags; not run yet: it isn't installed on the test PC.)
    async fn codex(&self, dir: &Path, system: &str, prompt: &str) -> Result<String> {
        let whole = agent_prompt(system, prompt, &[]);
        let answer = dir.join(format!("codex-{}-{}.txt", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
        let mut args: Vec<String> = ["exec", "--skip-git-repo-check", "--ephemeral", "--sandbox", "read-only", "--color", "never", "-o"].iter().map(|s| s.to_string()).collect();
        args.push(answer.display().to_string());
        for feature in ["shell_tool", "browser_use", "in_app_browser", "apps", "plugins"] {
            args.push("-c".into());
            args.push(format!("features.{feature}=false"));
        }
        if self.model != "default" {
            args.push("-m".into());
            args.push(self.model.clone());
        }
        args.push("-".into());
        let mut cmd = command(&self.exe, &args, dir);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().with_context(|| format!("couldn't start {}", self.exe.display()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(whole.as_bytes()).await?;
            drop(stdin);
        }
        let out = tokio::time::timeout(TURN, child.wait_with_output()).await.map_err(|_| anyhow!("no answer in {} s", TURN.as_secs()))??;
        let reply = std::fs::read_to_string(&answer).unwrap_or_default();
        let _ = std::fs::remove_file(&answer);
        if !out.status.success() || reply.trim().is_empty() {
            bail!("{}", explain(Kind::Codex, &format!("{}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout))));
        }
        Ok(reply.trim().to_string())
    }
}

/// A model you can pick for a plan brain (Settings › Brain).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanModel {
    pub id: String,
    pub label: String,
    /// What it's good for, when there's something to say.
    pub note: String,
    pub rec: bool,
}

/// The model a plan brain starts on.
pub fn default_model(kind: Kind) -> &'static str {
    match kind {
        Kind::Claude => "sonnet",
        Kind::Cursor => "composer-2.5",
        Kind::Codex => "default",
    }
}

/// The models you can pick for a plan brain, the recommended ones first. Cursor's come from its own list (it changes
/// often); Codex has no list to ask, so it's its own default.
pub async fn models(kind: Kind) -> Vec<PlanModel> {
    let m = |id: &str, label: &str, note: &str, rec: bool| PlanModel { id: id.into(), label: label.into(), note: note.into(), rec };
    match kind {
        Kind::Claude => vec![
            m("sonnet", "Sonnet (recommended)", "16 of 16 on her real-hands test, the quickest: about 24 s a job.", true),
            m("haiku", "Haiku", "Quickest and lightest on your plan. Not measured on her jobs yet.", true),
            m("opus", "Opus", "The smartest; uses up your plan the fastest.", false),
        ],
        Kind::Codex => vec![m("default", "Codex's default", "Whatever your Codex is set to.", true)],
        Kind::Cursor => {
            let listed = match find(kind) {
                Some(exe) => run(Kind::Cursor, &exe, &["models".to_string()], &folder().unwrap_or_else(|_| std::env::temp_dir()), None).await.unwrap_or_default(),
                None => String::new(),
            };
            let mut out = vec![
                m("composer-2.5", "Composer 2.5 (recommended)", "16 of 16 on her real-hands test seeing for itself, about 37 s a job. Cheap on your plan.", true),
                m("auto", "Auto", "Cursor picks the model for each step: 14 of 16 on her real-hands test, about 48 s a job.", true),
                m("composer-2.5-fast", "Composer 2.5 Fast", "15 of 16 on her real-hands test, about 34 s a job: the quickest of Cursor's.", true),
            ];
            for (id, label) in cursor_models(&listed) {
                if !out.iter().any(|x| x.id == id) {
                    out.push(m(&id, &label, "", false));
                }
            }
            out
        }
    }
}

/// `cursor-agent models` as (id, label): lines like "composer-2.5 - Composer 2.5" among its colours and headings.
fn cursor_models(listed: &str) -> Vec<(String, String)> {
    let mut clean = String::with_capacity(listed.len());
    let mut chars = listed.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // A colour code: up to its closing letter.
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() && n != '[' {
                    break;
                }
            }
        } else if !matches!(c, '\u{200b}'..='\u{200d}' | '\u{feff}') {
            clean.push(c);
        }
    }
    clean
        .lines()
        .filter_map(|l| l.trim().split_once(" - "))
        .filter(|(id, _)| !id.is_empty() && !id.contains(' '))
        .map(|(id, label)| (id.to_string(), label.replace("(current, default)", "").replace("(current)", "").replace("(default)", "").split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect()
}

/// Runs the app hidden in her folder with `input` on stdin, and gives back what it printed. A `.cmd` (npm's, Cursor's)
/// runs through cmd.exe; none of the arguments are your words, so nothing you say reaches a command line.
async fn run(kind: Kind, exe: &Path, args: &[String], dir: &Path, input: Option<String>) -> Result<String> {
    let mut cmd = command(exe, args, dir);
    if kind == Kind::Cursor
        && let Some(home) = cursor_home()
    {
        cmd.env("USERPROFILE", &home).env("HOME", &home);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().with_context(|| format!("couldn't start {}", exe.display()))?;
    if let (Some(mut stdin), Some(text)) = (child.stdin.take(), input) {
        stdin.write_all(text.as_bytes()).await?;
        drop(stdin);
    }
    let out = tokio::time::timeout(TURN, child.wait_with_output()).await.map_err(|_| anyhow!("no answer in {} s", TURN.as_secs()))??;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if stdout.trim().is_empty() {
        bail!("{}", explain(kind, &String::from_utf8_lossy(&out.stderr)));
    }
    Ok(stdout)
}

/// The app run hidden in her folder, an API key in the environment out of its way, gone when dropped.
fn command(exe: &Path, args: &[String], dir: &Path) -> tokio::process::Command {
    let is_cmd = exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd"));
    let mut cmd = if is_cmd {
        let mut c = tokio::process::Command::new("cmd.exe");
        c.arg("/d").arg("/s").arg("/c").arg(exe).args(args);
        c
    } else {
        let mut c = tokio::process::Command::new(exe);
        c.args(args);
        c
    };
    // And Git Bash's own marks: started from it, Cursor's agent writes its hooks for PowerShell and runs them in bash, and every
    // tool it tries is refused with a syntax error.
    cmd.current_dir(dir).kill_on_drop(true);
    for variable in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "SHELL", "MSYSTEM", "EXEPATH"] {
        cmd.env_remove(variable);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd
}

/// What went wrong, in words you can act on.
fn explain(kind: Kind, said: &str) -> String {
    let low = said.to_lowercase();
    if low.contains("login") || low.contains("log in") || low.contains("not logged") || low.contains("authenticat") || low.contains("invalid api key") {
        format!("{} isn't signed in: open a terminal, run {} and sign in", app(kind), match kind {
            Kind::Claude => "claude",
            Kind::Cursor => "cursor-agent login",
            Kind::Codex => "codex login",
        })
    } else if low.contains("limit") || low.contains("usage") || low.contains("quota") {
        format!("your plan's limit for now is used up ({})", short(said))
    } else {
        format!("{} said: {}", app(kind), short(said))
    }
}

fn short(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 300 { format!("{}…", s.chars().take(300).collect::<String>()) } else { s.to_string() }
}

/// A tiny run when the plan is picked, so a missing sign-in shows straight away instead of at her first job.
pub async fn check(kind: Kind, model: &str) -> Result<()> {
    let brain = PlanBrain::new("check", kind, model)?;
    let req = Req { system: "Reply with OK and nothing else.", msgs: &[Msg::user("OK?")], tools: &[], max_tokens: 20, effort: Effort::Low };
    let r = brain.chat(req).await?;
    if r.text.is_empty() {
        bail!("{} answered nothing", app(kind));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_models_by_their_server_names() {
        let listed: Vec<String> = ["default[]", "composer-2.5[fast=true]", "claude-sonnet-5-5[context=300k,reasoning_effort=high]"].iter().map(|s| s.to_string()).collect();
        assert_eq!(acp_model("auto", &listed).as_deref(), Some("default[]"));
        assert_eq!(acp_model("composer-2.5-fast", &listed).as_deref(), Some("composer-2.5[fast=true]"));
        assert_eq!(acp_model("composer-2.5", &listed).as_deref(), Some("composer-2.5[fast=true]"));
        assert_eq!(acp_model("claude-sonnet-5-5", &listed).as_deref(), Some("claude-sonnet-5-5[context=300k,reasoning_effort=high]"));
        assert_eq!(acp_model("gpt-9", &listed), None);
    }

    /// The real thing: a job of several steps on Cursor's server, a picture in one of them, each step timed.
    /// `cargo test -p capri-brain cursor_server_steps -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "calls Cursor"]
    async fn cursor_server_steps_are_quick() {
        use base64::Engine;
        let model = std::env::var("CAPRI_CURSOR_MODEL").unwrap_or_else(|_| "composer-2.5".into());
        let brain = PlanBrain::new(&format!("cursor/{model}"), Kind::Cursor, &model).unwrap();
        let started = Instant::now();
        brain.warm().await;
        println!("warm: {:.1} s (at her start, in the background)", started.elapsed().as_secs_f32());
        let tools = vec![
            capri_hands::ToolSpec { name: "open".into(), description: "Open an app by name".into(), parameters: json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }) },
            capri_hands::ToolSpec { name: "look".into(), description: "A picture of a window".into(), parameters: json!({ "type": "object", "properties": { "window": { "type": "string" } } }) },
        ];
        let system = "You work the user's PC with your tools. Do the job step by step; answer in one short line when it's done.";
        let mut msgs = vec![Msg::user("Request: open Steam, then look at its window and tell me the main colour you see.")];
        let art = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/icons/128x128.png");
        let picture = base64::engine::general_purpose::STANDARD.encode(std::fs::read(art).unwrap());
        for step in 0..5 {
            let started = Instant::now();
            let r = brain.chat(Req { system, msgs: &msgs, tools: &tools, max_tokens: 400, effort: Effort::Low }).await.unwrap();
            println!("step {step}: {:.1} s, calls {:?}, said {:?}", started.elapsed().as_secs_f32(), r.calls.iter().map(|c| c.name.clone()).collect::<Vec<_>>(), r.text.chars().take(80).collect::<String>());
            msgs.push(Msg::assistant(&r));
            if r.calls.is_empty() {
                return;
            }
            for c in &r.calls {
                let mut m = Msg::tool(c, if c.name == "look" { "here's Steam as a picture" } else { "ok: started Steam; its window is up" }, false);
                if c.name == "look" {
                    m.images.push(crate::llm::Image { mime: "image/png".into(), base64: picture.clone() });
                }
                msgs.push(m);
            }
        }
        panic!("no answer in 5 steps");
    }

    /// The real thing: Cursor's agent, signed in on this PC, opening a picture she hands it. `cargo test -p capri-brain cursor_opens -- --ignored`
    #[tokio::test]
    #[ignore = "calls Cursor"]
    async fn cursor_opens_a_picture_it_is_given() {
        use base64::Engine;
        let art = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/icons/128x128.png");
        let base64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(art).unwrap());
        let brain = PlanBrain::new("cursor/composer-2.5", Kind::Cursor, "composer-2.5").unwrap();
        let mut msg = Msg::user("In a few words, what's in the picture?");
        msg.images.push(crate::llm::Image { mime: "image/png".into(), base64 });
        let msgs = [msg];
        let started = Instant::now();
        let r = brain.chat(Req { system: "Say what you see.", msgs: &msgs, tools: &[], max_tokens: 200, effort: Effort::Low }).await.unwrap();
        println!("{:.1} s, {} tokens in: {}", started.elapsed().as_secs_f32(), r.usage.input, r.text);
        assert!(!r.text.trim().is_empty());
    }

    #[test]
    fn a_plan_brain_does_not_inherit_git_bash() {
        let cmd = command(Path::new("agent.exe"), &[], Path::new("."));
        let removed: Vec<String> = cmd.as_std().get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_string_lossy().to_uppercase()).collect();
        for variable in ["SHELL", "MSYSTEM", "EXEPATH"] {
            assert!(removed.iter().any(|r| r == variable), "{variable} would reach it");
        }
    }

    #[test]
    fn all_the_arguments_in_one_called_argument() {
        let (_, calls) = parse("<invoke name=\"scroll\"><parameter name=\"argument\">{\"direction\":\"bottom\",\"window\":\"Gym\"}</parameter></invoke>");
        assert_eq!(calls[0].args, json!({ "direction": "bottom", "window": "Gym" }));
    }

    #[test]
    fn a_word_in_quotes_is_still_that_word() {
        let (_, calls) = parse("<invoke name=\"scroll\"><parameter name=\"direction\">\"bottom\"</parameter><parameter name=\"pages\">2</parameter><parameter name=\"window\">Gym</parameter></invoke>");
        assert_eq!(calls[0].args, json!({ "direction": "bottom", "pages": 2, "window": "Gym" }));
    }

    #[test]
    fn a_coding_agent_is_told_her_tools_are_text() {
        let with = agent_prompt("Be Capri.\n\n# Your tools\n- click", "[User]\nplay it", &[]);
        assert!(with.starts_with("You are being run by a program"), "first thing it reads");
        assert!(with.ends_with("A block you write is run for you.)"), "last thing it reads");
        // A plain question (her check that it's signed in) has no tools to explain.
        assert_eq!(agent_prompt("Reply with OK.", "[User]\nOK?", &[]), "Reply with OK.\n\n# The conversation so far\n[User]\nOK?");
        let shown = agent_prompt("Be Capri.\n\n# Your tools\n- look", "[Tool result: look]\nhere", &["picture-1.png".to_string()]);
        assert!(shown.contains("picture-1.png") && shown.ends_with("A block you write is run for you.)"), "told which picture to open");
    }

    #[test]
    fn only_a_dropped_connection_is_tried_again() {
        assert!(passing("Cursor's agent said: read ECONNRESET"));
        assert!(!passing("your plan's limit for now is used up"));
        assert!(!passing("Cursor's agent isn't signed in: open a terminal, run cursor-agent login and sign in"));
    }

    #[test]
    fn cursors_list_of_models() {
        let listed = "\u{1b}[2K\u{1b}[GAvailable models\n\nauto - Auto (current, default)\ncomposer-2.5 - Composer 2.5\ngrok-4.7-low-fast - Grok 4.7  Low Fast\u{200b}\u{200b}\n\nTip: use --model <id>\n";
        let got = cursor_models(listed);
        assert_eq!(got[0], ("auto".to_string(), "Auto".to_string()));
        assert_eq!(got[1].0, "composer-2.5");
        assert_eq!(got[2], ("grok-4.7-low-fast".to_string(), "Grok 4.7 Low Fast".to_string()));
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn tool_calls_are_read_from_the_words() {
        let (words, calls) = parse("Opening it.\n<tool_call>{\"name\": \"open\", \"arguments\": {\"name\": \"Discord\"}}</tool_call>\n<tool_call>```json\n{\"name\": \"media\", \"arguments\": {\"action\": \"pause\"}}\n```</tool_call>");
        assert_eq!(words, "Opening it.");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "open");
        assert_eq!(calls[0].args["name"], "Discord");
        assert_eq!(calls[1].args["action"], "pause");
        let (words, calls) = parse("Done, it's open.");
        assert_eq!((words.as_str(), calls.len()), ("Done, it's open.", 0));
        // Claude's own habit.
        let (words, calls) = parse("<function_calls>\n<invoke name=\"media\">\n<parameter name=\"action\">pause</parameter>\n</invoke>\n<invoke name=\"click\">\n<parameter name=\"target\">the fifth video</parameter>\n<parameter name=\"nth\">5</parameter>\n</invoke>\n</function_calls>");
        assert_eq!(words, "");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].args["action"], "pause");
        assert_eq!(calls[1].args["nth"], 5);
        assert_eq!(calls[1].args["target"], "the fifth video");
        assert_eq!(calls[1].id, "call_1");
    }

    #[test]
    fn the_conversation_reads_in_order() {
        let call = Call { id: "c".into(), name: "open".into(), args: json!({ "name": "Steam" }), extra: Value::Null };
        let reply = Reply { text: String::new(), calls: vec![call.clone()], raw: None, stop: Stop::ToolUse, usage: Usage::default() };
        let t = transcript(&[Msg::user("open steam"), Msg::assistant(&reply), Msg::tool(&call, "ok: opened Steam", false)]);
        assert!(t.contains("[User]\nopen steam") && t.contains("<tool_call>") && t.contains("[Tool result: open]\nok: opened Steam") && t.ends_with("[Your next step]\n"));
    }
}
