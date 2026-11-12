//! Local models through Ollama's own API (`/api/chat`): unlike its OpenAI-compatible endpoint, it lets us set the
//! context size and how long the model stays loaded, and unload it when a game starts.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::llm::{Call, Model, Msg, Reply, Req, Role, Stop, Usage};

pub struct Ollama {
    id: String,
    name: String,
    host: String,
    http: reqwest::Client,
}

/// Enough for her tools, the situation and a few steps; KV cache for this fits next to an 8B model in 12 GB.
pub const NUM_CTX: u32 = 12288;

impl Ollama {
    pub fn new(id: &str, name: &str) -> Self {
        let host = std::env::var("OLLAMA_HOST").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "http://127.0.0.1:11434".into());
        let host = if host.starts_with("http") { host } else { format!("http://{host}") };
        Self { id: id.into(), name: name.into(), host: host.trim_end_matches('/').into(), http: super::http(Duration::from_secs(180)) }
    }

    /// Starts `ollama serve` in the background (no window) when Ollama is on this PC, and waits for it to answer.
    async fn start_server(&self) -> bool {
        if !(self.host.contains("127.0.0.1") || self.host.contains("localhost")) {
            return false;
        }
        tracing::info!("Ollama isn't running: starting it");
        let mut cmd = std::process::Command::new("ollama");
        cmd.arg("serve").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP: Ollama gets a hidden console of its own, which the model
            // runners it starts inherit, and Ctrl+C in her terminal doesn't reach it. (DETACHED_PROCESS left it with
            // no console at all, so every runner opened a visible terminal window.)
            cmd.creation_flags(0x0800_0000 | 0x0000_0200);
        }
        if let Err(e) = cmd.spawn() {
            tracing::warn!("couldn't start Ollama: {e}");
            return false;
        }
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if self.http.get(format!("{}/api/version", self.host)).send().await.is_ok() {
                return true;
            }
        }
        false
    }

    fn messages(system: &str, msgs: &[Msg]) -> Vec<Value> {
        let mut out = vec![json!({ "role": "system", "content": system })];
        for m in msgs {
            out.push(match m.role {
                Role::User => {
                    let mut v = json!({ "role": "user", "content": m.text });
                    if !m.images.is_empty() {
                        v["images"] = json!(m.images.iter().map(|i| i.base64.clone()).collect::<Vec<_>>());
                    }
                    v
                }
                Role::Assistant => {
                    let calls: Vec<Value> = m.calls.iter().map(|c| json!({ "function": { "name": c.name, "arguments": c.args } })).collect();
                    let mut v = json!({ "role": "assistant", "content": m.text });
                    if !calls.is_empty() {
                        v["tool_calls"] = json!(calls);
                    }
                    v
                }
                Role::Tool => json!({ "role": "tool", "content": m.text, "tool_name": m.tool_name }),
            });
        }
        out
    }
}

#[async_trait]
impl Model for Ollama {
    fn id(&self) -> &str {
        &self.id
    }

    fn local(&self) -> bool {
        // A ":cloud" model runs on Ollama's servers.
        !self.name.ends_with(":cloud") && !self.name.contains("-cloud")
    }

    async fn chat(&self, req: Req<'_>) -> Result<Reply> {
        let tools: Vec<Value> = req.tools.iter().map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters } })).collect();
        let mut body = json!({
            "model": self.name,
            "messages": Self::messages(req.system, req.msgs),
            "stream": false,
            // Long: reloading costs 12 s on the next request, and it's unloaded anyway the moment a game starts.
            "keep_alive": "2h",
            "options": { "temperature": 0, "num_ctx": NUM_CTX, "num_predict": req.max_tokens },
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        let url = format!("{}/api/chat", self.host);
        let res = match self.http.post(&url).json(&body).send().await {
            Ok(r) => r,
            // Not running (closed for a game, say): start it once and try again.
            Err(e) if e.is_connect() && self.start_server().await => self.http.post(&url).json(&body).send().await.context("Ollama started but isn't answering")?,
            Err(e) => return Err(e).context("Ollama isn't running and couldn't be started"),
        };
        let status = res.status();
        let v: Value = res.json().await.context("Ollama sent something unreadable")?;
        if !status.is_success() {
            bail!("Ollama: {}", v["error"].as_str().unwrap_or(&status.to_string()));
        }
        let msg = &v["message"];
        let mut calls = Vec::new();
        for (i, c) in msg["tool_calls"].as_array().into_iter().flatten().enumerate() {
            let f = &c["function"];
            let args = match &f["arguments"] {
                Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                v => v.clone(),
            };
            calls.push(Call { id: format!("call_{i}"), name: f["name"].as_str().unwrap_or_default().to_string(), args, extra: Value::Null });
        }
        let text = strip_think(msg["content"].as_str().unwrap_or_default());
        let stop = if !calls.is_empty() {
            Stop::ToolUse
        } else if v["done_reason"].as_str() == Some("length") {
            Stop::MaxTokens
        } else {
            Stop::Done
        };
        Ok(Reply {
            text,
            calls,
            raw: None,
            stop,
            usage: Usage { input: v["prompt_eval_count"].as_u64().unwrap_or(0), output: v["eval_count"].as_u64().unwrap_or(0), cached: 0 },
        })
    }

    async fn unload(&self) {
        let body = json!({ "model": self.name, "keep_alive": 0 });
        if let Err(e) = self.http.post(format!("{}/api/generate", self.host)).json(&body).send().await {
            tracing::warn!("unloading {}: {e}", self.name);
        }
    }
}

/// Some local models write their reasoning inline in <think>…</think>; it's not part of the answer.
fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</think>".len(), "");
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn think_is_stripped() {
        assert_eq!(super::strip_think("<think>hmm</think>\nPaused."), "Paused.");
        assert_eq!(super::strip_think("Done."), "Done.");
    }
}
