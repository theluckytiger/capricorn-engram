//! Any OpenAI-compatible chat API: OpenAI, OpenRouter, Groq, Gemini, DeepSeek, Mistral, NVIDIA, LM Studio,
//! llama.cpp, vLLM.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::llm::{Call, Model, Msg, Reply, Req, Role, Stop, Usage};

pub struct OpenAiCompatible {
    id: String,
    base: String,
    name: String,
    key: Option<String>,
    local: bool,
    http: reqwest::Client,
}

/// How long a rate limit says to wait, from its message ("Please try again in 2.5s" from Groq, "Please retry in
/// 37.6s" from Gemini, "Retry after 39s"): whole seconds, rounded up.
fn retry_after(msg: &str) -> Option<u64> {
    let low = msg.to_lowercase();
    let at = ["retry after ", "try again in ", "retry in "].iter().find_map(|w| low.find(w).map(|i| i + w.len()))?;
    let n: String = low[at..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let secs: f64 = n.trim_end_matches('.').parse().ok()?;
    let unit = low[at + n.len()..].trim_start();
    // "12h34m35s" is a day's quota spent: hours, not seconds (she waited 12 s and asked again, three times).
    let secs = if unit.starts_with("ms") {
        secs / 1000.0
    } else if unit.starts_with('h') {
        secs * 3600.0
    } else if unit.starts_with('m') {
        secs * 60.0
    } else {
        secs
    };
    Some(secs.ceil() as u64)
}

impl OpenAiCompatible {
    pub fn new(id: &str, base: &str, name: &str, key: Option<String>, local: bool) -> Self {
        // A server on this PC may be loading the model (slow the first time); a cloud one that hasn't answered in 40 s
        // isn't going to, and her local brain takes over.
        let wait = Duration::from_secs(if local { 120 } else { 40 });
        Self { id: id.into(), base: base.trim_end_matches('/').into(), name: name.into(), key, local, http: super::http(wait) }
    }

    fn messages(system: &str, msgs: &[Msg]) -> Vec<Value> {
        let mut out = vec![json!({ "role": "system", "content": system })];
        for m in msgs {
            out.push(match m.role {
                Role::User if m.images.is_empty() => json!({ "role": "user", "content": m.text }),
                Role::User => {
                    let mut parts = vec![json!({ "type": "text", "text": m.text })];
                    for i in &m.images {
                        parts.push(json!({ "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", i.mime, i.base64) } }));
                    }
                    json!({ "role": "user", "content": parts })
                }
                Role::Assistant => {
                    let mut v = json!({ "role": "assistant", "content": if m.text.is_empty() { Value::Null } else { json!(m.text) } });
                    if !m.calls.is_empty() {
                        v["tool_calls"] = json!(m
                            .calls
                            .iter()
                            .map(|c| {
                                let mut call = json!({ "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.args.to_string() } });
                                if !c.extra.is_null() {
                                    call["extra_content"] = c.extra.clone();
                                }
                                call
                            })
                            .collect::<Vec<_>>());
                    }
                    v
                }
                Role::Tool => json!({ "role": "tool", "tool_call_id": m.call_id, "content": m.text }),
            });
            // A tool's picture (look) can't ride in a tool message here: it comes right after, as one from the user.
            if m.role == Role::Tool && !m.images.is_empty() {
                let mut parts = vec![json!({ "type": "text", "text": format!("The picture {} gave:", m.tool_name.as_deref().unwrap_or("the tool")) })];
                for i in &m.images {
                    parts.push(json!({ "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", i.mime, i.base64) } }));
                }
                out.push(json!({ "role": "user", "content": parts }));
            }
        }
        out
    }
}

#[async_trait]
impl Model for OpenAiCompatible {
    fn id(&self) -> &str {
        &self.id
    }

    fn sees(&self) -> bool {
        let id = self.id.to_lowercase();
        !self.local && ["gemini", "-vl", "vision", "llama-4", "gpt-4o", "gpt-5", "gpt-4.1", "claude", "omni"].iter().any(|w| id.contains(w))
    }

    fn local(&self) -> bool {
        self.local
    }

    async fn chat(&self, req: Req<'_>) -> Result<Reply> {
        let mut body = json!({
            "model": self.name,
            "messages": if self.local { Self::messages(req.system, req.msgs) } else { Self::messages(req.system, &crate::llm::lean(req.msgs)) },
            "max_tokens": req.max_tokens,
        });
        // Gemini thinks at length unless told not to (no answer in 90 s): low, or a little more for hard work.
        // Groq's gpt-oss reasons at medium unless told (about 6 s a step for her).
        if self.id.starts_with("gemini/") || self.id.contains("gpt-oss") {
            body["reasoning_effort"] = json!(if matches!(req.effort, crate::llm::Effort::Low) { "low" } else { "medium" });
        }
        // Nemotron thinks before every answer and spent all of a short line's tokens on it (an empty reply): her
        // spoken lines and small talk go without.
        if self.id.starts_with("nvidia/") && self.id.contains("nemotron") && req.tools.is_empty() && matches!(req.effort, crate::llm::Effort::Low) {
            body["chat_template_kwargs"] = json!({ "enable_thinking": false });
        }
        if !req.tools.is_empty() {
            body["tools"] = json!(req.tools.iter().map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters } })).collect::<Vec<_>>());
        }
        // A free tier's rate limit (Groq, Gemini: a few requests a minute) says how long to wait: she waits that out,
        // three times at most and a minute each at most, rather than failing. A day's quota used up fails at once.
        let mut tries = 0;
        let v: Value = loop {
            let mut rb = self.http.post(format!("{}/chat/completions", self.base)).json(&body);
            if let Some(k) = &self.key {
                rb = rb.bearer_auth(k);
            }
            let res = rb.send().await.with_context(|| format!("{} isn't answering", self.base))?;
            let status = res.status();
            let header = res.headers().get("retry-after").and_then(|h| h.to_str().ok()).and_then(|h| h.trim().parse::<f64>().ok()).map(|s| s.ceil() as u64);
            let body_text = res.text().await.context("unreadable answer")?;
            if status.is_success() {
                break serde_json::from_str(&body_text).context("unreadable answer")?;
            }
            // A provider busy for a moment (NVIDIA's free models answer 503 now and then): twice more, a few
            // seconds apart.
            if tries < 2 && matches!(status.as_u16(), 502..=504) {
                tries += 1;
                tracing::info!("{}: {status}, trying again", self.id);
                tokio::time::sleep(Duration::from_secs(2 * tries)).await;
                continue;
            }
            let v: Value = serde_json::from_str(&body_text).unwrap_or(Value::Null);
            // Gemini wraps its error in a list.
            let err = if v.is_array() { &v[0]["error"] } else { &v["error"] };
            let msg = err["message"].as_str().or(err.as_str()).unwrap_or("").to_string();
            if tries < 3
                && (status.as_u16() == 402 || status.as_u16() == 429)
                && let Some(secs) = retry_after(&msg).or(header).filter(|s| *s <= 60)
            {
                tries += 1;
                tracing::info!("{}: {msg} (waiting {secs} s)", self.id);
                tokio::time::sleep(Duration::from_secs(secs + 1)).await;
                continue;
            }
            bail!("{} {status}: {msg}", self.id);
        };
        let choice = &v["choices"][0];
        let msg = &choice["message"];
        let calls = msg["tool_calls"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, c)| Call {
                id: c["id"].as_str().map(String::from).unwrap_or_else(|| format!("call_{i}")),
                name: c["function"]["name"].as_str().unwrap_or_default().to_string(),
                args: match &c["function"]["arguments"] {
                    Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                    other => other.clone(),
                },
                extra: c["extra_content"].clone(),
            })
            .collect::<Vec<_>>();
        let stop = match choice["finish_reason"].as_str() {
            _ if !calls.is_empty() => Stop::ToolUse,
            Some("length") => Stop::MaxTokens,
            Some("content_filter") => Stop::Refusal,
            Some("stop") | None => Stop::Done,
            Some(other) => Stop::Other(other.into()),
        };
        Ok(Reply {
            text: msg["content"].as_str().unwrap_or_default().trim().to_string(),
            calls,
            raw: None,
            stop,
            usage: Usage {
                input: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                output: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
                cached: v["usage"]["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn retry_after_in_a_message() {
        assert_eq!(super::retry_after("Free limit reached: 1 reply every 45 seconds per IP. Retry after 39s, or get a key"), Some(39));
        assert_eq!(super::retry_after("Payment required"), None);
        assert_eq!(super::retry_after("Rate limit reached for model `openai/gpt-oss-120b` on tokens per minute (TPM). Please try again in 2.53s."), Some(3));
        assert_eq!(super::retry_after("You exceeded your current quota. Please retry in 37.6s."), Some(38));
        assert_eq!(super::retry_after("Please try again in 450ms."), Some(1));
        assert_eq!(super::retry_after("Please try again in 7m12s."), Some(420));
        assert_eq!(super::retry_after("Please retry in 12h34m35.018759458s."), Some(43200));
    }
}
