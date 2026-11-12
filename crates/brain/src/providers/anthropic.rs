//! Claude through the Messages API (raw HTTP: there's no official Rust SDK).
//!
//! - API version 2026-10-21: no sampling parameters (they're rejected); depth is set with `output_config.effort`.
//! - Thinking is always on for current models: assistant content is replayed exactly as it came back.
//! - The system prompt and tools never change between requests, so the prefix is cached (top-level cache_control).
//! - A declined request falls back to another model inside the same call (`fallbacks: "default"`).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::llm::{Call, Effort, Model, Msg, Reply, Req, Role, Stop, Usage};

const URL: &str = "https://api.anthropic.com/v1/messages";
const VERSION: &str = "2026-10-21";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

pub struct Anthropic {
    id: String,
    name: String,
    key: String,
    http: reqwest::Client,
}

impl Anthropic {
    pub fn new(id: &str, name: &str, key: String) -> Self {
        Self { id: id.into(), name: name.into(), key, http: super::http(Duration::from_secs(300)) }
    }

    /// Tool results that answer one assistant turn go back together, in one user message.
    fn messages(msgs: &[Msg]) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        for m in msgs {
            match m.role {
                Role::User => {
                    let mut content: Vec<Value> = m
                        .images
                        .iter()
                        .map(|i| json!({ "type": "image", "source": { "type": "base64", "media_type": i.mime, "data": i.base64 } }))
                        .collect();
                    content.push(json!({ "type": "text", "text": m.text }));
                    match out.last_mut() {
                        Some(last) if last["role"] == "user" && last["content"].is_array() => last["content"].as_array_mut().unwrap().extend(content),
                        _ => out.push(json!({ "role": "user", "content": content })),
                    }
                }
                Role::Assistant => {
                    let content = match &m.raw {
                        Some(raw) => raw.clone(),
                        None => {
                            let mut c = Vec::new();
                            if !m.text.is_empty() {
                                c.push(json!({ "type": "text", "text": m.text }));
                            }
                            for call in &m.calls {
                                c.push(json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": call.args }));
                            }
                            json!(c)
                        }
                    };
                    out.push(json!({ "role": "assistant", "content": content }));
                }
                Role::Tool => {
                    let content = if m.images.is_empty() {
                        json!(m.text)
                    } else {
                        let mut parts = vec![json!({ "type": "text", "text": m.text })];
                        for i in &m.images {
                            parts.push(json!({ "type": "image", "source": { "type": "base64", "media_type": i.mime, "data": i.base64 } }));
                        }
                        json!(parts)
                    };
                    let block = json!({ "type": "tool_result", "tool_use_id": m.call_id, "content": content, "is_error": m.is_error });
                    let extend = out.last().is_some_and(|last| {
                        last["role"] == "user" && last["content"].as_array().is_some_and(|c| c.iter().all(|b| b["type"] == "tool_result"))
                    });
                    if extend {
                        out.last_mut().unwrap()["content"].as_array_mut().unwrap().push(block);
                    } else {
                        out.push(json!({ "role": "user", "content": [block] }));
                    }
                }
            }
        }
        out
    }
}

#[async_trait]
impl Model for Anthropic {
    fn id(&self) -> &str {
        &self.id
    }

    fn sees(&self) -> bool {
        true
    }

    fn local(&self) -> bool {
        false
    }

    async fn chat(&self, req: Req<'_>) -> Result<Reply> {
        let effort = match req.effort {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        };
        let mut body = json!({
            "model": self.name,
            "max_tokens": req.max_tokens.max(4096),
            "system": req.system,
            "messages": Self::messages(&crate::llm::lean(req.msgs)),
            "output_config": { "effort": effort },
            "cache_control": { "type": "ephemeral" },
            "fallbacks": "default",
        });
        if !req.tools.is_empty() {
            body["tools"] = json!(req.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.parameters })).collect::<Vec<_>>());
        }
        let res = self
            .http
            .post(URL)
            .header("x-api-key", &self.key)
            .header("anthropic-version", VERSION)
            .header("anthropic-beta", FALLBACK_BETA)
            .json(&body)
            .send()
            .await
            .context("the Claude API isn't answering")?;
        let status = res.status();
        let v: Value = res.json().await.context("unreadable answer from the Claude API")?;
        if !status.is_success() {
            let kind = v["error"]["type"].as_str().unwrap_or("error");
            let msg = v["error"]["message"].as_str().unwrap_or("");
            bail!("Claude API {status} ({kind}): {msg}");
        }
        let content = v["content"].as_array().cloned().unwrap_or_default();
        let mut text = String::new();
        let mut calls = Vec::new();
        for b in &content {
            match b["type"].as_str() {
                Some("text") => text.push_str(b["text"].as_str().unwrap_or_default()),
                Some("tool_use") => calls.push(Call { id: b["id"].as_str().unwrap_or_default().into(), name: b["name"].as_str().unwrap_or_default().into(), args: b["input"].clone(), extra: Value::Null }),
                _ => {}
            }
        }
        let stop = match v["stop_reason"].as_str() {
            Some("end_turn") | Some("stop_sequence") => Stop::Done,
            Some("tool_use") => Stop::ToolUse,
            Some("max_tokens") => Stop::MaxTokens,
            Some("refusal") => Stop::Refusal,
            Some(other) => Stop::Other(other.into()),
            None => Stop::Done,
        };
        let u = &v["usage"];
        Ok(Reply {
            text: text.trim().to_string(),
            calls,
            raw: Some(Value::Array(content)),
            stop,
            usage: Usage { input: u["input_tokens"].as_u64().unwrap_or(0), output: u["output_tokens"].as_u64().unwrap_or(0), cached: u["cache_read_input_tokens"].as_u64().unwrap_or(0) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Reply;

    #[test]
    fn tool_results_share_one_user_message() {
        let reply = Reply {
            text: String::new(),
            calls: vec![Call { id: "a".into(), name: "x".into(), args: json!({}), extra: Value::Null }, Call { id: "b".into(), name: "y".into(), args: json!({}), extra: Value::Null }],
            raw: None,
            stop: Stop::ToolUse,
            usage: Usage::default(),
        };
        let msgs = vec![Msg::user("hi"), Msg::assistant(&reply), Msg::tool(&reply.calls[0], "1", false), Msg::tool(&reply.calls[1], "2", true)];
        let out = Anthropic::messages(&msgs);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(out[2]["content"][1]["is_error"], true);
    }
}
