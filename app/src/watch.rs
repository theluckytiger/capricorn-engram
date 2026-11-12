//! Watching coding AIs at work: Claude Code, from its own transcripts in `~/.claude/projects` (read here, never sent
//! anywhere). Each action it takes, a command that fails, it going quiet to think, and the end of its turn (with a
//! summary) go to her overlay as `capri://coder`. Off in Settings › Privacy › "Watch coding AIs".

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize, Clone, Default)]
struct Coder {
    /// action, failed, quiet, done, answer, gone
    kind: &'static str,
    agent: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    /// It ran a command, or went online.
    command: bool,
    online: bool,
    /// Quiet: for how long. Done: how long the turn took.
    secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    /// Done: what the turn added up to.
    actions: u32,
    files: u32,
    errors: u32,
}

/// Today's missions (each turn a coding AI finished), for her mission report: kept in `missions.json`.
#[derive(Serialize, serde::Deserialize, Default, Clone)]
pub struct Missions {
    pub date: String,
    pub missions: u32,
    pub actions: u32,
    pub files: u32,
    pub errors: u32,
    /// The longest mission, seconds.
    pub longest: u64,
}

fn missions_path() -> Option<PathBuf> {
    capri_core::settings::data_dir().ok().map(|d| d.join("missions.json"))
}

/// Today's missions so far.
#[tauri::command]
pub fn mission_report() -> Missions {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let m: Missions = missions_path().and_then(|p| capri_core::read_json_text(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if m.date == today { m } else { Missions { date: today, ..Default::default() } }
}

fn record(c: &Coder) {
    let mut m = mission_report();
    m.missions += 1;
    m.actions += c.actions;
    m.files += c.files;
    m.errors += c.errors;
    m.longest = m.longest.max(c.secs);
    if let Some(p) = missions_path() {
        let _ = std::fs::write(p, serde_json::to_string_pretty(&m).unwrap_or_default());
    }
}

/// Its turn so far: since your last message to it.
#[derive(Default)]
struct Turn {
    actions: u32,
    files: HashSet<String>,
    errors: u32,
    since: Option<Instant>,
    /// What it's said since its last action: at the end of its turn, its answer.
    text: String,
}

struct Watching {
    file: PathBuf,
    offset: u64,
    partial: String,
    turn: Turn,
    last_line: Instant,
    /// The quiet stretch already spoken of (40 s, 2 min).
    quiet_said: u8,
    busy: bool,
}

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("coders".into())
        .spawn(move || {
            let Some(root) = directories::BaseDirs::new().map(|d| d.home_dir().join(".claude").join("projects")) else { return };
            let mut w: Option<Watching> = None;
            loop {
                std::thread::sleep(Duration::from_millis(700));
                if !app.state::<crate::App>().settings.lock().unwrap().privacy.watch_coders {
                    // Switched off while watching: what was over her head goes too.
                    if w.take().is_some() {
                        let _ = app.emit("capri://coder", Coder { kind: "gone", agent: "Claude Code", ..Default::default() });
                    }
                    continue;
                }
                let emit = |c: Coder| {
                    let _ = app.emit("capri://coder", c);
                };
                // The transcript being written now (a turn in the last minute and a half).
                let newest = newest(&root).filter(|(_, t)| t.elapsed().is_ok_and(|e| e < Duration::from_secs(90)));
                match (&mut w, newest) {
                    (Some(cur), Some((file, _))) if cur.file == file => {}
                    (_, Some((file, _))) => {
                        // A new one: from its end (what happened before isn't news).
                        let offset = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                        w = Some(Watching { file, offset, partial: String::new(), turn: Turn::default(), last_line: Instant::now(), quiet_said: 0, busy: false });
                    }
                    (Some(cur), None) => {
                        if cur.last_line.elapsed() > Duration::from_secs(150) {
                            emit(Coder { kind: "gone", agent: "Claude Code", ..Default::default() });
                            w = None;
                        }
                        continue;
                    }
                    (None, None) => continue,
                }
                let cur = w.as_mut().expect("watching");
                for line in read_new(cur) {
                    cur.last_line = Instant::now();
                    cur.quiet_said = 0;
                    for c in on_line(&line, &mut cur.turn, &mut cur.busy) {
                        if c.kind == "done" {
                            capri_core::journal::write("coder", serde_json::json!({ "agent": c.agent, "summary": c.summary }));
                            record(&c);
                        }
                        // Its answer only when you're looking at something else (not Claude, a terminal or an editor).
                        if c.kind == "answer" && !elsewhere() {
                            continue;
                        }
                        emit(c);
                    }
                }
                // Thinking a long while in the middle of a turn: worth a word at 40 s and at 2 min.
                let quiet = cur.last_line.elapsed().as_secs();
                if cur.busy && ((quiet >= 40 && cur.quiet_said == 0) || (quiet >= 120 && cur.quiet_said == 1)) {
                    cur.quiet_said += 1;
                    emit(Coder { kind: "quiet", agent: "Claude Code", secs: quiet, ..Default::default() });
                }
            }
        })
        .expect("coders thread");
}

/// Whether the window you're in is somewhere you wouldn't see Claude's answer: not Claude's own app, a terminal or a
/// code editor (where it may be running).
fn elsewhere() -> bool {
    #[cfg(windows)]
    {
        let me = std::process::id();
        let Some(front) = capri_hands::platform::list_windows().into_iter().find(|w| w.foreground) else { return false };
        if front.pid == me {
            return false;
        }
        let exe = front.exe.to_lowercase();
        const WHERE_IT_RUNS: &[&str] = &["claude", "windowsterminal", "cmd", "powershell", "pwsh", "conhost", "wezterm-gui", "alacritty", "code", "cursor", "windsurf", "idea64", "rustrover64", "pycharm64", "devenv"];
        !WHERE_IT_RUNS.contains(&exe.as_str()) && !front.title.contains("Claude Code")
    }
    #[cfg(not(windows))]
    false
}

/// The most recently written transcript under `root` (one folder per project), and when.
fn newest(root: &Path) -> Option<(PathBuf, SystemTime)> {
    let mut best: Option<(PathBuf, SystemTime)> = None;
    for project in std::fs::read_dir(root).ok()?.flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            if let Ok(t) = f.metadata().and_then(|m| m.modified())
                && best.as_ref().is_none_or(|(_, b)| t > *b)
            {
                best = Some((p, t));
            }
        }
    }
    best
}

/// Whole lines written since last time.
fn read_new(w: &mut Watching) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(&w.file) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len < w.offset {
        w.offset = 0;
    }
    if len == w.offset || f.seek(SeekFrom::Start(w.offset)).is_err() {
        return vec![];
    }
    let mut buf = Vec::new();
    let _ = f.take(4 * 1024 * 1024).read_to_end(&mut buf);
    w.offset += buf.len() as u64;
    w.partial.push_str(&String::from_utf8_lossy(&buf));
    let mut lines: Vec<String> = w.partial.split('\n').map(String::from).collect();
    w.partial = lines.pop().unwrap_or_default();
    lines.into_iter().filter(|l| !l.trim().is_empty()).collect()
}

/// What one transcript line means for her.
fn on_line(line: &str, turn: &mut Turn, busy: &mut bool) -> Vec<Coder> {
    let Ok(e) = serde_json::from_str::<Value>(line) else { return vec![] };
    let msg = &e["message"];
    let mut out = Vec::new();
    match e["type"].as_str() {
        Some("user") => match &msg["content"] {
            // Your message to it: a new turn.
            Value::String(_) => new_turn(turn, busy),
            Value::Array(items) => {
                if items.iter().any(|i| i["type"] == "text") && !items.iter().any(|i| i["type"] == "tool_result") {
                    new_turn(turn, busy);
                }
                for r in items.iter().filter(|i| i["type"] == "tool_result" && i["is_error"] == true) {
                    turn.errors += 1;
                    out.push(Coder { kind: "failed", agent: "Claude Code", detail: Some(error_line(&r["content"])), ..Default::default() });
                }
            }
            _ => {}
        },
        Some("assistant") => {
            for item in msg["content"].as_array().into_iter().flatten() {
                if item["type"] == "text" {
                    let t = item["text"].as_str().unwrap_or("").trim();
                    if !t.is_empty() {
                        if !turn.text.is_empty() {
                            turn.text.push_str("\n\n");
                        }
                        turn.text.push_str(t);
                    }
                    continue;
                }
                if item["type"] != "tool_use" {
                    continue;
                }
                turn.text.clear();
                let name = item["name"].as_str().unwrap_or("");
                let Some(c) = action(name, &item["input"], turn) else { continue };
                *busy = true;
                turn.since.get_or_insert_with(Instant::now);
                turn.actions += 1;
                out.push(c);
            }
            // Its answer, for her wide panel when you're on another window (the overlay decides).
            let answer = (msg["stop_reason"] == "end_turn" && !turn.text.is_empty()).then(|| {
                let t: String = turn.text.chars().take(8000).collect();
                Coder { kind: "answer", agent: "Claude Code", detail: Some(t), ..Default::default() }
            });
            if msg["stop_reason"] == "end_turn" && turn.actions > 0 {
                let secs = turn.since.map(|s| s.elapsed().as_secs()).unwrap_or(0);
                let mut parts = vec![format!("{} action{}", turn.actions, if turn.actions == 1 { "" } else { "s" })];
                if !turn.files.is_empty() {
                    parts.push(format!("{} file{} changed", turn.files.len(), if turn.files.len() == 1 { "" } else { "s" }));
                }
                if turn.errors > 0 {
                    parts.push(format!("{} error{}", turn.errors, if turn.errors == 1 { "" } else { "s" }));
                }
                if secs >= 60 {
                    parts.push(format!("{} min", secs / 60));
                }
                out.push(Coder { kind: "done", agent: "Claude Code", secs, summary: Some(parts.join(", ")), actions: turn.actions, files: turn.files.len() as u32, errors: turn.errors, ..Default::default() });
                *turn = Turn::default();
                *busy = false;
            }
            if let Some(a) = answer {
                turn.text.clear();
                out.push(a);
            }
        }
        _ => {}
    }
    out
}

fn new_turn(turn: &mut Turn, busy: &mut bool) {
    *turn = Turn { since: Some(Instant::now()), ..Default::default() };
    *busy = true;
}

/// One action, for her ticker: the tool and a short detail. `None` for its own bookkeeping (task lists, tool loading).
fn action(name: &str, input: &Value, turn: &mut Turn) -> Option<Coder> {
    let short = |s: &str, n: usize| {
        let first = s.lines().next().unwrap_or("").trim();
        if first.chars().count() > n { format!("{}…", first.chars().take(n).collect::<String>()) } else { first.to_string() }
    };
    let file = |key: &str| input[key].as_str().map(|p| Path::new(p).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
    let tool = name.rsplit("__").next().unwrap_or(name);
    if matches!(tool, "TodoWrite" | "ToolSearch" | "update_plan" | "wrap_up" | "mark_chapter") {
        return None;
    }
    let mut c = Coder { kind: "action", agent: "Claude Code", tool: Some(tool.to_string()), ..Default::default() };
    match tool {
        "Bash" | "PowerShell" => {
            c.command = true;
            c.detail = input["command"].as_str().map(|s| short(s, 48));
        }
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            c.detail = file("file_path").or_else(|| file("notebook_path"));
            if let Some(f) = input["file_path"].as_str().or(input["notebook_path"].as_str()) {
                turn.files.insert(f.to_string());
            }
        }
        "Read" => c.detail = file("file_path"),
        "Grep" | "Glob" => c.detail = input["pattern"].as_str().map(|s| short(s, 40)),
        "WebFetch" => {
            c.online = true;
            c.detail = input["url"].as_str().map(|u| u.split('/').nth(2).unwrap_or(u).to_string());
        }
        "WebSearch" => {
            c.online = true;
            c.detail = input["query"].as_str().map(|s| short(s, 40));
        }
        "Task" | "Agent" => c.detail = input["description"].as_str().map(|s| short(s, 40)),
        _ => {
            c.online = name.contains("Browser") || name.contains("chrome");
            c.detail = input["action"].as_str().or(input["url"].as_str()).map(|s| short(s, 40));
        }
    }
    Some(c)
}

/// The line of an error that matters: the first that says error or failed, else the first.
fn error_line(content: &Value) -> String {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|i| i["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    let pick = text.lines().find(|l| {
        let l = l.to_lowercase();
        l.contains("error") || l.contains("failed") || l.contains("denied")
    });
    let line = pick.or_else(|| text.lines().find(|l| !l.trim().is_empty())).unwrap_or("it failed").trim();
    if line.chars().count() > 90 { format!("{}…", line.chars().take(90).collect::<String>()) } else { line.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn() {
        let (mut turn, mut busy) = (Turn::default(), false);
        let lines = [
            r#"{"type":"user","message":{"content":"fix the build"}}"#,
            r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo build\nmore"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"   Compiling x\nerror[E0308]: mismatched types"}]}}"#,
            r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Edit","input":{"file_path":"C:\\x\\src\\main.rs"}}]}}"#,
            r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Fixed."}]}}"#,
        ];
        let got: Vec<Coder> = lines.iter().flat_map(|l| on_line(l, &mut turn, &mut busy)).collect();
        let kinds: Vec<&str> = got.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, ["action", "failed", "action", "done", "answer"]);
        assert_eq!(got[4].detail.as_deref(), Some("Fixed."));
        assert_eq!(got[0].detail.as_deref(), Some("cargo build"));
        assert!(got[0].command);
        assert_eq!(got[1].detail.as_deref(), Some("error[E0308]: mismatched types"));
        assert_eq!(got[2].detail.as_deref(), Some("main.rs"));
        assert_eq!(got[3].summary.as_deref(), Some("2 actions, 1 file changed, 1 error"));
        assert!(!busy);
    }
}
