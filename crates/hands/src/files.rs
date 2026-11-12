//! `files`: finding files on this PC by name (and kind) in your own folders, newest first, or by the words in them
//! (Windows' own search index), and opening one or showing it in its folder.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

pub struct Files;

/// Folders never worth walking: caches, build output, other programs' insides.
const SKIP: &[&str] = &["node_modules", ".git", "appdata", "target", "$recycle.bin", "windows", "program files", "program files (x86)", "programdata", ".cache", ".cargo", ".rustup", "__pycache__", ".venv", "venv"];
/// Programs and scripts: opening one runs it.
const RUNS: &[&str] = &["exe", "bat", "cmd", "ps1", "vbs", "msi", "scr", "com", "js", "jar", "reg", "lnk"];

fn kind_of(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "document" => &["pdf", "doc", "docx", "odt", "rtf", "txt", "md", "xls", "xlsx", "csv", "ppt", "pptx", "odp", "ods", "pages", "epub"],
        "pdf" => &["pdf"],
        "image" => &["png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "svg", "tiff", "psd"],
        "video" => &["mp4", "mkv", "mov", "avi", "webm", "wmv", "flv", "m4v"],
        "audio" => &["mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "opus"],
        "archive" => &["zip", "rar", "7z", "tar", "gz", "xz", "bz2"],
        "code" => &["rs", "py", "js", "ts", "tsx", "jsx", "c", "cpp", "h", "cs", "java", "go", "rb", "php", "html", "css", "json", "toml", "yaml", "yml", "sh", "lua"],
        "program" => &["exe", "msi", "lnk"],
        _ => return None,
    })
}

#[async_trait]
impl Tool for Files {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "files".into(),
            description: "Files on this PC, in the user's own folders (Desktop, Documents, Downloads, Pictures, Videos, \
                          Music, OneDrive). `find`: by `name` (words in the file's name; spelling as said) and/or `kind` \
                          (document, pdf, image, video, audio, archive, code, program), newest first, with their \
                          folders. `search` with `words`: only when they describe what's written INSIDE a file (\"the doc where I \
                          wrote about the budget\", \"the file that mentions Porto\"), from Windows' search index; a file \
                          they name (\"my resume\") is `find` by name. `open` with `path` (from find): opens it. `show` with `path`: opens its folder with \
                          it selected. Say what you found and where; open one only when asked to."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["find", "search", "open", "show"] },
                    "words": { "type": "string" },
                    "name": { "type": "string" },
                    "kind": { "type": "string", "enum": ["document", "pdf", "image", "video", "audio", "archive", "code", "program", "any"] },
                    "path": { "type": "string" }
                },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        match args.get("action").and_then(Value::as_str) {
            Some("open") => {
                let ext = args.get("path").and_then(Value::as_str).and_then(|p| Path::new(p).extension()).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                if RUNS.contains(&ext.as_str()) { Risk::Risky } else { Risk::Normal }
            }
            Some("show") => Risk::Normal,
            _ => Risk::Read,
        }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        match action.as_str() {
            "find" => {
                let name: String = opt_arg(&args, "name").unwrap_or_default();
                let kind: String = opt_arg(&args, "kind").unwrap_or_else(|| "any".into());
                if name.trim().is_empty() && kind == "any" {
                    return Outcome::failed("give a name or a kind to look for");
                }
                let found = tokio::task::spawn_blocking(move || find(&name, kind_of(&kind), Duration::from_millis(2500))).await.unwrap_or_default();
                if found.is_empty() {
                    return Outcome::failed("nothing like that in Desktop, Documents, Downloads, Pictures, Videos, Music or OneDrive");
                }
                let lines: Vec<String> = found.iter().take(8).map(|(p, t)| format!("{} — in {} ({})", file_name(p), folder(p), when(*t))).collect();
                let paths: Vec<String> = found.iter().take(8).map(|(p, _)| p.display().to_string()).collect();
                Outcome::ok(format!("found {}{}: {}", found.len(), if found.len() > 8 { " (newest 8)" } else { "" }, lines.join("; "))).with(json!({ "paths": paths }))
            }
            "search" => {
                let words: String = opt_arg(&args, "words").or_else(|| opt_arg(&args, "name")).unwrap_or_default();
                let words = search_words(&words);
                if words.is_empty() {
                    return Outcome::failed("give words to look for inside files");
                }
                let shown = words.join(" ");
                #[cfg(windows)]
                let found = tokio::task::spawn_blocking(move || indexed(&words)).await.unwrap_or_else(|e| Err(e.to_string()));
                #[cfg(not(windows))]
                let found: Result<Vec<(PathBuf, SystemTime)>, String> = Err("not on this OS yet".into());
                match found {
                    Ok(f) if f.is_empty() => Outcome::failed(format!("no file in your folders has \"{shown}\" in it (Windows' index has none; a file it hasn't indexed yet won't show)")),
                    Ok(f) => {
                        let lines: Vec<String> = f.iter().take(8).map(|(p, t)| format!("{} — in {} ({})", file_name(p), folder(p), when(*t))).collect();
                        let paths: Vec<String> = f.iter().take(8).map(|(p, _)| p.display().to_string()).collect();
                        Outcome::ok(format!("{} file{} with \"{shown}\" in {}: {}", f.len(), if f.len() == 1 { "" } else { "s" }, if f.len() == 1 { "it" } else { "them" }, lines.join("; "))).with(json!({ "paths": paths }))
                    }
                    Err(e) => Outcome::failed(format!("Windows' search index didn't answer: {e}")),
                }
            }
            "open" | "show" => {
                let path: String = match arg(&args, "path") {
                    Ok(p) => p,
                    Err(o) => return o,
                };
                if !Path::new(&path).exists() {
                    return Outcome::failed(format!("{path} isn't there (any more)"));
                }
                if ctx.dry_run {
                    return Outcome::ok(format!("(dry run) would {action} {path}"));
                }
                #[cfg(windows)]
                {
                    let r = if action == "open" {
                        crate::pc::shell_open(&path)
                    } else {
                        std::process::Command::new("explorer.exe").arg(format!("/select,{path}")).spawn().map(|_| ())
                    };
                    match r {
                        Ok(()) => Outcome::ok(if action == "open" { format!("opened {}", file_name(Path::new(&path))) } else { format!("showing {} in {}", file_name(Path::new(&path)), folder(Path::new(&path))) }),
                        Err(e) => Outcome::failed(format!("couldn't: {e}")),
                    }
                }
                #[cfg(not(windows))]
                Outcome::failed("not on this OS yet")
            }
            other => Outcome::failed(format!("unknown action \"{other}\"")),
        }
    }
}

/// Where to look: the user's folders (and OneDrive's copies of them).
fn roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(u) = directories::UserDirs::new() {
        for d in [u.desktop_dir(), u.document_dir(), u.download_dir(), u.picture_dir(), u.video_dir(), u.audio_dir()].into_iter().flatten() {
            out.push(d.to_path_buf());
        }
        let one = u.home_dir().join("OneDrive");
        if one.is_dir() {
            out.push(one);
        }
    }
    // Documents moved into OneDrive is inside it: walked once.
    let all = out.clone();
    out.retain(|r| !all.iter().any(|o| o != r && r.starts_with(o)));
    out.dedup();
    out
}

/// Files whose names hold every word of `name` (and of `kind`), newest first; walks for at most `budget`.
pub fn find(name: &str, kind: Option<&[&str]>, budget: Duration) -> Vec<(PathBuf, SystemTime)> {
    let words: Vec<String> = name.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && !matches!(*w, "my" | "the" | "a" | "file" | "files" | "called" | "named")).map(String::from).collect();
    let start = Instant::now();
    let mut found = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = roots().into_iter().map(|r| (r, 0)).collect();
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if start.elapsed() > budget || seen > 200_000 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            seen += 1;
            let file_name = e.file_name().to_string_lossy().to_lowercase();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                if depth < 7 && !file_name.starts_with('.') && !SKIP.contains(&file_name.as_str()) {
                    stack.push((e.path(), depth + 1));
                }
                continue;
            }
            let p = e.path();
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            if kind.is_some_and(|k| !k.contains(&ext.as_str())) {
                continue;
            }
            let stem: String = file_name.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
            if words.iter().all(|w| stem.contains(w.as_str())) {
                let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                found.push((p, modified));
            }
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.1));
    found
}

/// The words to look for inside files: letters and digits only (they go into the index's query), at most six.
fn search_words(said: &str) -> Vec<String> {
    said.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect::<String>())
        .filter(|w| w.chars().count() >= 2 && !["the", "a", "an", "and", "or", "of", "in", "on", "my", "file", "files", "document", "doc"].contains(&w.to_lowercase().as_str()))
        .take(6)
        .collect()
}

/// Files in the user's own folders whose text has every one of `words`, from Windows' search index, newest first.
#[cfg(windows)]
fn indexed(words: &[String]) -> Result<Vec<(PathBuf, SystemTime)>, String> {
    use std::os::windows::process::CommandExt;
    let home = directories::UserDirs::new().map(|u| u.home_dir().display().to_string().replace('\\', "/")).ok_or("no home folder")?;
    // Only letters, digits, - and _ get here (search_words), so nothing in them can break out of the query.
    let contains = words.iter().map(|w| format!("\"\"{w}*\"\"")).collect::<Vec<_>>().join(" AND ");
    let script = format!(
        "$ErrorActionPreference='Stop';$c=New-Object -ComObject ADODB.Connection;$c.Open(\"Provider=Search.CollatorDSO;Extended Properties='Application=Windows';\");\
         $r=$c.Execute(\"SELECT TOP 40 System.ItemPathDisplay FROM SYSTEMINDEX WHERE SCOPE='file:{home}' AND CONTAINS(*,'{contains}') ORDER BY System.DateModified DESC\");\
         while(-not $r.EOF){{[Console]::WriteLine($r.Fields.Item(0).Value);$r.MoveNext()}}"
    );
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("it failed").to_string());
    }
    let skip = |p: &Path| p.components().any(|c| SKIP.contains(&c.as_os_str().to_string_lossy().to_lowercase().as_str()));
    let mut found: Vec<(PathBuf, SystemTime)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| PathBuf::from(l.trim()))
        .filter(|p| p.is_file() && !skip(p))
        .map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            (p, t)
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.1));
    Ok(found)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
}

/// The folder, said the short way ("Downloads", "Documents › Taxes").
fn folder(p: &Path) -> String {
    let parent = p.parent().unwrap_or(p);
    let home = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf());
    match home.as_ref().and_then(|h| parent.strip_prefix(h).ok()) {
        Some(rel) if !rel.as_os_str().is_empty() => rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join(" › "),
        _ => parent.display().to_string(),
    }
}

fn when(t: SystemTime) -> String {
    let t: chrono::DateTime<chrono::Local> = t.into();
    let now = chrono::Local::now();
    if t.date_naive() == now.date_naive() {
        format!("today {}", t.format("%H:%M"))
    } else if (now - t).num_days() < 7 {
        t.format("%A").to_string()
    } else {
        t.format("%-d %b %Y").to_string()
    }
}
