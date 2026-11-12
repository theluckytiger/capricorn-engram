//! Her journal: everything she heard, was asked, did and said, one JSON line per event, in
//! `%APPDATA%\capricorn-engram\logs\journal-YYYY-MM-DD.jsonl`. It's how her mistakes get found: read a session back
//! with `python tools/journal.py` (today's) or `python tools/journal.py <file>`.
//!
//! Next to it, `llm-YYYY-MM-DD.jsonl` holds every model call in full (what was sent, what came back), too big for
//! the journal: `python tools/journal.py --llm` shows them under each request.
//!
//! Only on this PC. Recordings of what she heard (dev builds) sit next to it in `logs\audio\`, the last 60.
//!
//! Started with `CAPRI_LOG_EVERYTHING=1` she's being tested, and logs everything (`everything()`): every click and setting in
//! her windows, every call between her pages and her core, what she acted out, and each picture shown to a model
//! (`logs\shots\`). `python tools/journal.py --session --all` reads a test session back.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

struct Open {
    log: String,
    day: String,
    file: File,
}

static FILES: Mutex<Vec<Open>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);
static PREFIX: OnceLock<String> = OnceLock::new();

/// She's being tested (`CAPRI_LOG_EVERYTHING=1`): everything goes in. Off for anyone else.
pub fn everything() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("CAPRI_LOG_EVERYTHING").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// `%APPDATA%\capricorn-engram\logs` (next to her `config` folder).
pub fn dir() -> PathBuf {
    crate::settings::data_dir().map(|d| d.parent().map(|p| p.join("logs")).unwrap_or(d.join("logs"))).unwrap_or_else(|_| PathBuf::from("logs"))
}

/// Puts this process's logs in their own files (`<prefix>-llm-…`), so a test run doesn't mix into her own. Call once,
/// before anything is written.
pub fn set_prefix(prefix: &str) {
    let _ = PREFIX.set(prefix.to_string());
}

/// A new request id, to tie what she heard, the steps she took and her answer together.
pub fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Writes one event to her journal: `kind` plus the fields of `data` (an object), stamped with the local time.
pub fn write(kind: &str, data: Value) {
    write_to("journal", kind, data);
}

/// The same, into the log called `log` (`<log>-YYYY-MM-DD.jsonl`).
pub fn write_to(log: &str, kind: &str, data: Value) {
    let now = chrono::Local::now();
    let day = now.format("%Y-%m-%d").to_string();
    let mut line = json!({ "t": now.format("%H:%M:%S%.3f").to_string(), "kind": kind });
    if let (Some(obj), Value::Object(extra)) = (line.as_object_mut(), data) {
        obj.extend(extra);
    }
    let mut files = FILES.lock().unwrap();
    let i = match files.iter().position(|o| o.log == log) {
        Some(i) if files[i].day == day => i,
        found => {
            let d = dir();
            let _ = std::fs::create_dir_all(&d);
            let name = format!("{}{log}-{day}.jsonl", PREFIX.get().map_or("", |p| p.as_str()));
            let file = match OpenOptions::new().create(true).append(true).open(d.join(name)) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("{log} log: {e}");
                    return;
                }
            };
            let open = Open { log: log.to_string(), day, file };
            match found {
                Some(i) => {
                    files[i] = open;
                    i
                }
                None => {
                    files.push(open);
                    files.len() - 1
                }
            }
        }
    };
    let _ = writeln!(files[i].file, "{line}");
}

/// A crash in native code (a segmentation fault) isn't a panic, and would leave nothing behind: it says what it was too.
#[cfg(windows)]
pub fn catch_crashes() {
    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        record: *const ExceptionRecord,
        address: *const std::ffi::c_void,
    }
    #[repr(C)]
    struct ExceptionPointers {
        record: *const ExceptionRecord,
        context: *const std::ffi::c_void,
    }
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(filter: Option<unsafe extern "system" fn(*const ExceptionPointers) -> i32>) -> *const std::ffi::c_void;
    }
    unsafe extern "system" fn on_crash(info: *const ExceptionPointers) -> i32 {
        // SAFETY: Windows hands us valid pointers for the length of this call.
        let (code, address) = unsafe {
            let record = &*(*info).record;
            (record.code, record.address as usize)
        };
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        write("native crash", json!({ "code": format!("{code:#010x}"), "address": format!("{address:#x}"), "thread": thread, "backtrace": std::backtrace::Backtrace::force_capture().to_string() }));
        0 // carry on to Windows' own handling: she still goes down
    }
    // SAFETY: it only installs a function that lives as long as the program.
    unsafe {
        SetUnhandledExceptionFilter(Some(on_crash));
    }
}
