//! When she's being tested (started with `CAPRI_LOG_EVERYTHING=1`), everything that happens in her windows goes
//! into her journal next to what she heard, did and said: one timeline to read back. The pages send what happened
//! here (`ui/src/shared/log.ts`). Panics and what a save changed go in always: they cost nothing and nobody else reads
//! them (the journal never leaves this PC).

use capri_core::journal;
use serde_json::{Map, Value, json};

/// Whether the pages should report what happens in them.
#[tauri::command]
pub fn log_everything() -> bool {
    journal::everything()
}

/// What happened in a page, a few at a time.
#[tauri::command]
pub fn log_ui(page: String, events: Vec<Value>) {
    if !journal::everything() {
        return;
    }
    for event in events {
        let mut line = json!({ "page": page });
        if let (Some(obj), Value::Object(event)) = (line.as_object_mut(), event) {
            obj.extend(event);
        }
        journal::write("ui", line);
    }
}

/// A crash says where it was, in the journal, before she goes.
pub fn catch_panics() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        journal::write("panic", json!({ "message": info.to_string(), "thread": thread, "backtrace": std::backtrace::Backtrace::force_capture().to_string() }));
        before(info);
    }));
}

/// The top of a test session: which code she's running and how she's set up.
pub fn session(settings: &capri_core::Settings) {
    if !journal::everything() {
        return;
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let git = |args: &[&str]| {
        let mut cmd = std::process::Command::new("git");
        cmd.args(args).current_dir(&repo);
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
        cmd.output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    };
    let changed = git(&["status", "--porcelain"]).lines().count();
    journal::write("session", json!({ "commit": git(&["log", "-1", "--format=%h %s"]), "uncommitted_files": changed, "settings": settings }));
}

/// What differs between two settings, as `{ "brain.smart": [old, new] }`.
pub fn changes(before: &Value, after: &Value) -> Value {
    fn walk(path: &str, a: &Value, b: &Value, out: &mut Map<String, Value>) {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for key in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                    let inner = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                    walk(&inner, x.get(key).unwrap_or(&Value::Null), y.get(key).unwrap_or(&Value::Null), out);
                }
            }
            _ if a != b => {
                out.insert(path.to_string(), json!([a, b]));
            }
            _ => {}
        }
    }
    let mut out = Map::new();
    walk("", before, after, &mut out);
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::changes;
    use serde_json::json;

    #[test]
    fn only_what_changed_is_listed() {
        let a = json!({ "brain": { "fast": "x", "smart": "claudecode/sonnet" }, "voice": { "on": true } });
        let b = json!({ "brain": { "fast": "x", "smart": "cursor/auto" }, "voice": { "on": true } });
        assert_eq!(changes(&a, &b), json!({ "brain.smart": ["claudecode/sonnet", "cursor/auto"] }));
    }
}
