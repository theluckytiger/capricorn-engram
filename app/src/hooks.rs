//! Claude Code calling her: its Notification hook (it needs your permission, or it's waiting on you) runs a one-line
//! `curl` to a listener here, on this PC only (127.0.0.1). That never shows in its transcript, which is all her watcher
//! reads, so without it she couldn't tell you. The hook is added to Claude Code's settings from her Settings › Privacy
//! ("Let Claude Code call me"), with a copy of the file kept first.

use std::io::{Read, Write};
use std::net::TcpListener;

use tauri::{AppHandle, Emitter};

/// Where she listens for it (this PC only).
pub const PORT: u16 = 47219;

/// The hook's command, as it goes in Claude Code's settings (curl comes with Windows 10 and later).
fn command() -> String {
    format!("curl -s -m 2 -X POST --data-binary @- http://127.0.0.1:{PORT}/claude")
}

pub fn start(app: AppHandle) {
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("Claude Code's hook: can't listen on {PORT}: {e}");
            return;
        }
    };
    let _ = std::thread::Builder::new().name("claude-hook".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            // The request: its head, then a body of Content-Length (64 KB at most).
            while let Ok(n) = stream.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(head_end) = find(&buf, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
                    let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    if buf.len() >= head_end + 4 + len || buf.len() > 65536 {
                        break;
                    }
                }
            }
            let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
            let text = String::from_utf8_lossy(&buf);
            if !text.starts_with("POST /claude") {
                continue;
            }
            let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
            let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let message = v["message"].as_str().unwrap_or("it's waiting on you").to_string();
            capri_core::journal::write("claude needs you", serde_json::json!({ "message": message, "event": v["hook_event_name"] }));
            let _ = app.emit("capri://coder", serde_json::json!({ "kind": "needs_you", "agent": "Claude Code", "detail": message, "window": claude_window(), "command": false, "online": false, "secs": 0 }));
        }
    });
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Where Claude is on screen (physical px: left, top, right, bottom): its app's window, or a terminal it runs in.
fn claude_window() -> Option<[i32; 4]> {
    #[cfg(windows)]
    {
        let wins = capri_hands::platform::list_windows();
        let me = std::process::id();
        wins.iter()
            .filter(|w| w.pid != me && !w.minimized)
            .find(|w| w.app.eq_ignore_ascii_case("claude") || w.title.contains("Claude Code"))
            .map(|w| w.rect)
    }
    #[cfg(not(windows))]
    None
}

/// Her Settings › "Let Claude Code call me": adds the hook to Claude Code's own settings (`~/.claude/settings.json`),
/// keeping a copy of the file as it was. Says what it did.
#[tauri::command(async)]
pub fn connect_claude() -> Result<String, String> {
    let home = directories::BaseDirs::new().ok_or("no home folder")?.home_dir().to_path_buf();
    let path = home.join(".claude").join("settings.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("Claude Code's settings aren't readable JSON: {e}"))?;
    if text.contains(&format!("127.0.0.1:{PORT}")) {
        return Ok("Already connected: Claude Code calls her when it needs you.".into());
    }
    if path.exists() {
        std::fs::copy(&path, path.with_extension("json.before-capri")).map_err(|e| format!("couldn't keep a copy of Claude Code's settings: {e}"))?;
    }
    let entry = serde_json::json!({ "matcher": "", "hooks": [{ "type": "command", "command": command() }] });
    let hooks = v.as_object_mut().ok_or("Claude Code's settings aren't an object")?.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let list = hooks.as_object_mut().ok_or("its hooks aren't an object")?.entry("Notification").or_insert_with(|| serde_json::json!([]));
    list.as_array_mut().ok_or("its Notification hooks aren't a list")?.push(entry);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| format!("couldn't write Claude Code's settings: {e}"))?;
    capri_core::journal::write("setup", serde_json::json!({ "claude_hook": "added" }));
    Ok("Connected. When Claude Code needs you, she'll come and tell you (new Claude Code sessions).".into())
}
