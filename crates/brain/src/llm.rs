//! A conversation with a model, in words every provider can be translated to.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::Result;
use async_trait::async_trait;
use capri_hands::ToolSpec;
use serde_json::{Value, json};

/// How much of an older step's result an online brain is sent again (it read all of it when it was new).
const LEAN: usize = 1200;

/// What an online brain is sent of a job so far, each step: pictures from the newest look only (each is ~1500
/// tokens, and an older one shows a screen that's gone), and long results from before the last two steps cut to
/// their start. A free tier counts every token of every step; her own models get it all.
pub fn lean(msgs: &[Msg]) -> Vec<Msg> {
    let newest_picture = msgs.iter().rposition(|m| !m.images.is_empty());
    let results: Vec<usize> = msgs.iter().enumerate().filter(|(_, m)| m.role == Role::Tool).map(|(i, _)| i).collect();
    let recent = &results[results.len().saturating_sub(2)..];
    msgs.iter()
        .enumerate()
        .map(|(i, m)| {
            let mut m = m.clone();
            if Some(i) != newest_picture && !m.images.is_empty() {
                m.images.clear();
                m.text.push_str(" [its picture was shown then; look again for the screen now]");
            }
            let n = m.text.chars().count();
            if m.role == Role::Tool && !recent.contains(&i) && n > LEAN + 100 {
                m.text = format!("{}… [{} more characters, shown in full earlier]", m.text.chars().take(LEAN).collect::<String>(), n - LEAN);
            }
            m
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// A tool's result, answering one of the assistant's calls.
    Tool,
}

#[derive(Debug, Clone)]
pub struct Image {
    pub mime: String,
    pub base64: String,
}

#[derive(Debug, Clone)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub args: Value,
    /// What the provider attached to the call that must go back with it (Gemini's thought signature: without it
    /// the next turn is refused with 400).
    pub extra: Value,
}

#[derive(Debug, Clone)]
pub struct Msg {
    pub role: Role,
    pub text: String,
    pub images: Vec<Image>,
    /// Assistant: the tools it called.
    pub calls: Vec<Call>,
    /// Tool: which call this answers, and which tool it was.
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    pub is_error: bool,
    /// Assistant: the provider's own content, replayed verbatim to the same provider (Anthropic needs its thinking
    /// blocks back unchanged).
    pub raw: Option<Value>,
}

impl Msg {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, text: text.into(), images: vec![], calls: vec![], call_id: None, tool_name: None, is_error: false, raw: None }
    }
    pub fn tool(call: &Call, text: impl Into<String>, is_error: bool) -> Self {
        Self { role: Role::Tool, text: text.into(), images: vec![], calls: vec![], call_id: Some(call.id.clone()), tool_name: Some(call.name.clone()), is_error, raw: None }
    }
    pub fn assistant(reply: &Reply) -> Self {
        Self { role: Role::Assistant, text: reply.text.clone(), images: vec![], calls: reply.calls.clone(), call_id: None, tool_name: None, is_error: false, raw: reply.raw.clone() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
}

pub struct Req<'a> {
    pub system: &'a str,
    pub msgs: &'a [Msg],
    pub tools: &'a [ToolSpec],
    pub max_tokens: u32,
    pub effort: Effort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    Done,
    ToolUse,
    MaxTokens,
    /// The provider declined (safety); the text says what it said.
    Refusal,
    Other(String),
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<Call>,
    pub raw: Option<Value>,
    pub stop: Stop,
    pub usage: Usage,
}

#[async_trait]
pub trait Model: Send + Sync {
    /// "ollama/qwen3-vl:8b-instruct"
    fn id(&self) -> &str;
    /// Runs on this PC (nothing leaves it).
    fn local(&self) -> bool;
    /// Sees pictures well enough to work a window from one (the look and click_at tools are for these).
    fn sees(&self) -> bool {
        false
    }
    async fn chat(&self, req: Req<'_>) -> Result<Reply>;
    /// Frees the GPU (local models); a no-op for APIs.
    async fn unload(&self) {}
}

/// Asks `model`, and writes the whole exchange to her model log (`logs\llm-YYYY-MM-DD.jsonl`): the messages sent
/// (screenshots as a count, never their pixels), the tools offered, the reply, how long it took and the tokens.
/// `role` says which part of her asked ("agent", "speak", "chat"); `request` ties it to her journal.
pub async fn ask(model: &dyn Model, req: Req<'_>, role: &str, request: u64) -> Result<Reply> {
    let (system, msgs, tools, max_tokens) = (req.system, req.msgs, req.tools, req.max_tokens);
    let t = Instant::now();
    let r = model.chat(req).await;
    let mut entry = json!({
        "request": request,
        "role": role,
        "model": model.id(),
        "ms": t.elapsed().as_millis(),
        "max_tokens": max_tokens,
        "system": system_ref(system),
        "tools": tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        "msgs": msgs.iter().map(msg_json).collect::<Vec<_>>(),
    });
    match &r {
        Ok(reply) => {
            entry["reply"] = json!({
                "text": reply.text,
                "calls": reply.calls.iter().map(|c| json!({ "name": c.name, "args": c.args })).collect::<Vec<_>>(),
                "stop": format!("{:?}", reply.stop),
                "usage": { "input": reply.usage.input, "output": reply.usage.output, "cached": reply.usage.cached },
            })
        }
        Err(e) => entry["error"] = json!(format!("{e:#}")),
    }
    capri_core::journal::write_to("llm", "call", entry);
    r
}

fn msg_json(m: &Msg) -> Value {
    let role = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut v = json!({ "role": role, "text": m.text });
    if !m.calls.is_empty() {
        v["calls"] = json!(m.calls.iter().map(|c| json!({ "name": c.name, "args": c.args })).collect::<Vec<_>>());
    }
    if let Some(t) = &m.tool_name {
        v["tool"] = json!(t);
    }
    if m.is_error {
        v["error"] = json!(true);
    }
    if !m.images.is_empty() {
        v["images"] = json!(m.images.len());
        if capri_core::journal::everything() {
            v["shots"] = json!(m.images.iter().filter_map(keep_shot).collect::<Vec<_>>());
        }
    }
    v
}

/// A picture shown to a model, kept while she's being tested (`logs\shots\`, the newest 500; the same picture sent again
/// in a later step is the same file). Gives its path.
fn keep_shot(image: &Image) -> Option<String> {
    use base64::Engine;
    use std::hash::{Hash, Hasher};
    let dir = capri_core::journal::dir().join("shots");
    std::fs::create_dir_all(&dir).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    image.base64.hash(&mut h);
    let ext = image.mime.rsplit('/').next().unwrap_or("png");
    let path = dir.join(format!("{:016x}.{ext}", h.finish()));
    if !path.exists() {
        std::fs::write(&path, base64::engine::general_purpose::STANDARD.decode(&image.base64).ok()?).ok()?;
        let mut old: Vec<_> = std::fs::read_dir(&dir).ok()?.filter_map(|e| e.ok()).filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
        old.sort();
        for (_, p) in &old[..old.len().saturating_sub(500)] {
            let _ = std::fs::remove_file(p);
        }
    }
    Some(path.to_string_lossy().into_owned())
}

/// A system prompt goes in the model log in full the first time it's seen each day, then by its hash.
fn system_ref(system: &str) -> String {
    use std::hash::{Hash, Hasher};
    static SEEN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    system.hash(&mut h);
    let id = format!("{:016x}", h.finish());
    let key = format!("{} {id}", chrono::Local::now().format("%Y-%m-%d"));
    if SEEN.lock().unwrap().insert(key) {
        capri_core::journal::write_to("llm", "system", json!({ "hash": id, "text": system }));
    }
    id
}

#[cfg(test)]
mod lean_tests {
    use super::*;

    fn result(text: &str, picture: bool) -> Msg {
        let call = Call { id: "1".into(), name: "look".into(), args: json!({}), extra: Value::Null };
        let mut m = Msg::tool(&call, text, false);
        if picture {
            m.images.push(Image { mime: "image/png".into(), base64: "AAAA".into() });
        }
        m
    }

    #[test]
    fn old_pictures_and_long_old_results_are_left_behind() {
        let long = "x".repeat(5000);
        let msgs = vec![Msg::user("do it"), result(&long, true), result("short", true), result(&long, false), result("last", true)];
        let sent = lean(&msgs);
        assert_eq!(sent.iter().filter(|m| !m.images.is_empty()).count(), 1);
        assert!(!sent[4].images.is_empty());
        assert!(sent[1].text.len() < 1500 && sent[1].text.contains("more characters"));
        // The last two results go as they are.
        assert_eq!(sent[3].text, long);
        assert_eq!(sent[2].text, "short [its picture was shown then; look again for the screen now]");
    }
}
