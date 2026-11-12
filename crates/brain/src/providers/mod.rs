//! Turning a model id ("ollama/qwen3-vl:8b-instruct", "anthropic/claude-opus-5-5", "gemini/gemini-3-flash") into
//! something that can chat. Two wire formats cover everything: OpenAI-compatible chat (with Ollama's native API for
//! local models, which can also unload them) and Anthropic's Messages API.

mod anthropic;
mod ollama;
mod openai;
pub mod plan;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::llm::Model;

pub use anthropic::Anthropic;
pub use ollama::{NUM_CTX as OLLAMA_NUM_CTX, Ollama};
pub use openai::OpenAiCompatible;

/// An OpenAI-compatible service: its base URL, the env var its key comes from, whether it runs on this PC, what it's
/// called, and the page where a key for it is made.
struct Compat {
    prefix: &'static str,
    base: &'static str,
    key_env: Option<&'static str>,
    local: bool,
    name: &'static str,
    key_page: Option<&'static str>,
}

const COMPAT: &[Compat] = &[
    Compat { prefix: "lmstudio", base: "http://127.0.0.1:1234/v1", key_env: None, local: true, name: "LM Studio", key_page: None },
    Compat { prefix: "llamacpp", base: "http://127.0.0.1:8080/v1", key_env: None, local: true, name: "llama.cpp", key_page: None },
    Compat { prefix: "vllm", base: "http://127.0.0.1:8000/v1", key_env: None, local: true, name: "vLLM", key_page: None },
    Compat { prefix: "gemini", base: "https://generativelanguage.googleapis.com/v1beta/openai", key_env: Some("GEMINI_API_KEY"), local: false, name: "Google Gemini", key_page: Some("https://aistudio.google.com/apikey") },
    Compat { prefix: "groq", base: "https://api.groq.com/openai/v1", key_env: Some("GROQ_API_KEY"), local: false, name: "Groq", key_page: Some("https://console.groq.com/keys") },
    Compat { prefix: "openai", base: "https://api.openai.com/v1", key_env: Some("OPENAI_API_KEY"), local: false, name: "OpenAI", key_page: Some("https://platform.openai.com/api-keys") },
    Compat { prefix: "openrouter", base: "https://openrouter.ai/api/v1", key_env: Some("OPENROUTER_API_KEY"), local: false, name: "OpenRouter", key_page: Some("https://openrouter.ai/settings/keys") },
    Compat { prefix: "deepseek", base: "https://api.deepseek.com/v1", key_env: Some("DEEPSEEK_API_KEY"), local: false, name: "DeepSeek", key_page: Some("https://platform.deepseek.com/api_keys") },
    Compat { prefix: "mistral", base: "https://api.mistral.ai/v1", key_env: Some("MISTRAL_API_KEY"), local: false, name: "Mistral", key_page: Some("https://console.mistral.ai/api-keys") },
    Compat { prefix: "nvidia", base: "https://integrate.api.nvidia.com/v1", key_env: Some("NVIDIA_API_KEY"), local: false, name: "NVIDIA", key_page: Some("https://build.nvidia.com/settings/api-keys") },
];

/// Where an API key comes from: the OS keyring (set in her settings), else an environment variable.
pub type KeyLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub fn env_keys() -> KeyLookup {
    Arc::new(|provider: &str| {
        let var = match provider {
            "anthropic" => "ANTHROPIC_API_KEY",
            other => COMPAT.iter().find(|c| c.prefix == other).and_then(|c| c.key_env)?,
        };
        std::env::var(var).ok().filter(|k| !k.trim().is_empty())
    })
}

/// The cloud providers that take a key: (id, what it's called).
pub fn cloud_providers() -> Vec<(&'static str, &'static str)> {
    let mut out = vec![("anthropic", "Anthropic (Claude)")];
    for c in COMPAT.iter().filter(|c| c.key_env.is_some()) {
        out.push((c.prefix, c.name));
    }
    out
}

/// The environment variable a provider's key can come from.
pub fn key_env(provider: &str) -> Option<&'static str> {
    if provider == "anthropic" { Some("ANTHROPIC_API_KEY") } else { COMPAT.iter().find(|c| c.prefix == provider).and_then(|c| c.key_env) }
}

/// The page where a key for `provider` is made.
pub fn key_page(provider: &str) -> Option<&'static str> {
    if provider == "anthropic" {
        return Some("https://console.anthropic.com/settings/keys");
    }
    COMPAT.iter().find(|c| c.prefix == provider).and_then(|c| c.key_page)
}

/// Which plan brain a card in her settings means, if it's one.
pub fn plan_kind(provider: &str) -> Option<plan::Kind> {
    match provider {
        "claudecode" => Some(plan::Kind::Claude),
        "cursor" => Some(plan::Kind::Cursor),
        "codex" => Some(plan::Kind::Codex),
        _ => None,
    }
}

/// Her online brain from any provider, free key or paid: a good model your key can use there, for thinking and (if
/// it sees pictures) her eyes, as model ids. Picked from the provider's own list, so newer models are used as they
/// come, and each pick answers one small request first: a model swamped on a free tier (Gemini's newest gave no answer
/// in 90 s) is passed over, and a key with no credit says so now rather than at her first hard job.
pub async fn online_brain(provider: &str, key: &str, model: Option<&str>) -> Result<(String, Option<String>)> {
    // Your own plan: no key, the app is signed in; one tiny run on the model you picked shows it works.
    if let Some(kind) = plan_kind(provider) {
        let picked = model.filter(|m| !m.trim().is_empty());
        let mut name = picked.unwrap_or(plan::default_model(kind));
        if let Err(e) = plan::check(kind, name).await {
            // Cursor's list changes: the model she starts on may have gone, and its Auto is always there.
            if picked.is_some() || kind != plan::Kind::Cursor || plan::check(kind, "auto").await.is_err() {
                return Err(e);
            }
            name = "auto";
        }
        let id = format!("{provider}/{name}");
        // Claude Code and Cursor's agent take pictures from her; Codex hasn't been tried.
        return Ok((id.clone(), matches!(kind, plan::Kind::Claude | plan::Kind::Cursor).then_some(id)));
    }
    if provider == "ollamacloud" {
        return ollama_cloud().await;
    }
    if provider == "anthropic" {
        return anthropic_brain(key).await;
    }
    let Some(c) = COMPAT.iter().find(|c| c.prefix == provider && c.key_env.is_some()) else { bail!("unknown provider \"{provider}\"") };
    let r = http(Duration::from_secs(12)).get(format!("{}/models", c.base)).bearer_auth(key).send().await?;
    if !r.status().is_success() {
        bail!("{} said {}", c.name, r.status());
    }
    let v: serde_json::Value = r.json().await?;
    let ids = model_ids(&v);
    // The one you picked in Settings, or the best she knows of that your key can use.
    let picks = match model.filter(|m| !m.trim().is_empty()) {
        Some(m) => vec![m.to_string()],
        None => candidates(provider, &v, &ids),
    };
    if picks.is_empty() {
        bail!("none of {}'s models she knows how to use are on your key", c.name);
    }
    let mut tried = Vec::new();
    for m in picks.iter().take(if provider == "gemini" { 6 } else { 3 }) {
        match answers(c.base, key, m, provider == "gemini").await {
            Ok(()) => return Ok((format!("{provider}/{m}"), eyes_for(provider, &v, &ids, m).map(|e| format!("{provider}/{e}")))),
            Err(e) => tried.push(format!("{m}: {e}")),
        }
    }
    bail!("{} didn't answer ({})", c.name, tried.join("; "))
}

/// The ids in a provider's `/models` answer.
fn model_ids(v: &serde_json::Value) -> Vec<String> {
    v["data"].as_array().into_iter().flatten().filter(|m| m["active"].as_bool() != Some(false)).filter_map(|m| m["id"].as_str()).map(|id| id.trim_start_matches("models/").to_string()).collect()
}

/// The models a key can pick from in Settings, best first: the ones she knows work (she'd pick the first by herself).
pub async fn key_models(provider: &str, key: &str) -> Vec<plan::PlanModel> {
    let Some(c) = COMPAT.iter().find(|c| c.prefix == provider && c.key_env.is_some()) else { return Vec::new() };
    let Ok(r) = http(Duration::from_secs(12)).get(format!("{}/models", c.base)).bearer_auth(key).send().await else { return Vec::new() };
    let Ok(v) = r.json::<serde_json::Value>().await else { return Vec::new() };
    let note = |id: &str| match id {
        "nvidia/nemotron-3.5-super-vl-120b-a12b" => "13 of 16 on her real-hands test, about 22 s a job, the quickest online brain; it can see your screen.".to_string(),
        _ => String::new(),
    };
    candidates(provider, &v, &model_ids(&v)).into_iter().take(10).enumerate().map(|(i, id)| plan::PlanModel { label: if i == 0 { format!("{id} (recommended)") } else { id.clone() }, note: note(&id), rec: i < 3, id }).collect()
}

/// Ollama's free cloud models, through the Ollama on this PC once you're signed in to it (`ollama signin`): big open
/// models with nothing to download. The first of the strong ones that answers.
async fn ollama_cloud() -> Result<(String, Option<String>)> {
    let host = std::env::var("OLLAMA_HOST").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "http://127.0.0.1:11434".into());
    let host = if host.starts_with("http") { host } else { format!("http://{host}") };
    let mut tried = Vec::new();
    for m in ["gpt-oss:120b-cloud", "deepseek-v4.1-flash:cloud", "glm-5.3-flash:cloud", "minimax-m3.1-flash:cloud", "qwen3-coder:480b-cloud"] {
        let body = serde_json::json!({ "model": m, "messages": [{ "role": "user", "content": "Reply OK" }], "stream": false });
        let r = http(Duration::from_secs(30)).post(format!("{}/api/chat", host.trim_end_matches('/'))).json(&body).send().await;
        match r {
            Ok(r) if r.status().is_success() => return Ok((format!("ollama/{m}"), None)),
            Ok(r) => {
                let status = r.status();
                let said = r.text().await.unwrap_or_default();
                if status.as_u16() == 401 || said.to_lowercase().contains("unauthorized") {
                    bail!("you're not signed in to Ollama yet: open a terminal, run ollama signin, and pick this again");
                }
                tried.push(format!("{m}: {status}"));
            }
            Err(e) => tried.push(format!("{m}: {e}")),
        }
    }
    bail!("none of Ollama's cloud models answered ({})", tried.join("; "))
}

/// Claude: the newest Sonnet (the best at working a PC for the price), else Opus, else what there is; it sees.
async fn anthropic_brain(key: &str) -> Result<(String, Option<String>)> {
    let http = http(Duration::from_secs(20));
    let v: serde_json::Value = http.get("https://api.anthropic.com/v1/models").header("x-api-key", key).header("anthropic-version", "2023-06-01").send().await?.json().await?;
    // Newest first, as Anthropic lists them.
    let ids: Vec<String> = v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).map(String::from).collect();
    let pick = ["sonnet", "opus", "haiku"].iter().find_map(|f| ids.iter().find(|id| id.contains(f))).or(ids.first()).cloned().ok_or_else(|| anyhow::anyhow!("no Claude models on your key"))?;
    let body = serde_json::json!({ "model": pick, "max_tokens": 16, "messages": [{ "role": "user", "content": "Say ok." }] });
    let r = http.post("https://api.anthropic.com/v1/messages").header("x-api-key", key).header("anthropic-version", "2023-06-01").json(&body).send().await?;
    if !r.status().is_success() {
        let status = r.status();
        let v: serde_json::Value = r.json().await.unwrap_or_default();
        bail!("Anthropic said {status}: {}", v["error"]["message"].as_str().unwrap_or(""));
    }
    Ok((format!("anthropic/{pick}"), Some(format!("anthropic/{pick}"))))
}

/// Whether `model` answers a small request with a tool within ten seconds (Gemini at low effort: it thinks at length
/// otherwise).
async fn answers(base: &str, key: &str, model: &str, low_effort: bool) -> Result<()> {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": "Open Discord." }],
        "max_tokens": 200,
        "tools": [{ "type": "function", "function": { "name": "open", "description": "Open an app by name", "parameters": { "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] } } }]
    });
    if low_effort {
        body["reasoning_effort"] = "low".into();
    }
    let r = http(Duration::from_secs(10)).post(format!("{base}/chat/completions")).bearer_auth(key).json(&body).send().await.map_err(|_| anyhow::anyhow!("no answer in 10 s"))?;
    if !r.status().is_success() {
        let status = r.status();
        let v: serde_json::Value = r.json().await.unwrap_or_default();
        let err = if v.is_array() { &v[0]["error"] } else { &v["error"] };
        bail!("{status} {}", err["message"].as_str().unwrap_or("").chars().take(160).collect::<String>());
    }
    Ok(())
}

/// Gemini's Flash models on the key, best first: newest version, stable before preview, Lite last.
fn gemini_flashes(ids: &[String]) -> Vec<String> {
    let version = |id: &str| id.strip_prefix("gemini-").and_then(|r| r.split('-').next()).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    let odd = ["image", "tts", "live", "audio", "embedding", "thinking", "8b", "native", "exp", "computer", "robotics"];
    let mut list: Vec<String> = ids.iter().filter(|id| id.starts_with("gemini-") && id.contains("flash") && !odd.iter().any(|o| id.contains(o))).cloned().collect();
    list.sort_by(|a, b| a.contains("lite").cmp(&b.contains("lite")).then_with(|| version(b).total_cmp(&version(a))).then_with(|| a.contains("preview").cmp(&b.contains("preview"))).then_with(|| a.len().cmp(&b.len())));
    list
}

/// A provider's models worth using as her brain, best first (`v`: its raw model list, for what it says of each).
fn candidates(provider: &str, v: &serde_json::Value, ids: &[String]) -> Vec<String> {
    let have = |want: &[&str]| want.iter().filter(|w| ids.iter().any(|id| id == *w)).map(|w| w.to_string()).collect::<Vec<_>>();
    match provider {
        // Gemini: Flash (fast, sees pictures, the most free requests a day).
        "gemini" => gemini_flashes(ids),
        // Groq: open models that call tools well.
        "groq" => {
            let mut list = have(&["openai/gpt-oss-120b", "moonshotai/kimi-k2-instruct-0905", "moonshotai/kimi-k2-instruct", "llama-3.3-70b-versatile", "qwen/qwen3-32b", "openai/gpt-oss-20b"]);
            let more: Vec<String> = ids.iter().filter(|id| (id.contains("70b") || id.contains("120b")) && !list.contains(id)).cloned().collect();
            list.extend(more);
            list
        }
        // OpenAI: its newest GPT for chat and tools ("gpt-5.2", "gpt-5.2-mini": not audio, images, search, dated
        // snapshots), full size before mini.
        "openai" => {
            let plain = |id: &str| {
                let rest = id.strip_prefix("gpt-").unwrap_or("");
                let rest = rest.strip_suffix("-mini").unwrap_or(rest);
                !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
            };
            let version = |id: &str| id.trim_start_matches("gpt-").split('-').next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
            let mut list: Vec<String> = ids.iter().filter(|id| plain(id)).cloned().collect();
            list.sort_by(|a, b| version(b).total_cmp(&version(a)).then_with(|| a.contains("mini").cmp(&b.contains("mini"))));
            list
        }
        // OpenRouter: the newest of the strong families it carries.
        "openrouter" => {
            let mut models: Vec<(&str, i64)> = v["data"].as_array().into_iter().flatten().filter_map(|m| Some((m["id"].as_str()?, m["created"].as_i64().unwrap_or(0)))).collect();
            models.sort_by_key(|m| std::cmp::Reverse(m.1));
            ["anthropic/claude-sonnet", "google/gemini", "openai/gpt-", "deepseek/deepseek-chat"].iter().filter_map(|f| models.iter().find(|(id, _)| id.starts_with(f) && !id.contains("image") && !id.contains("audio")).map(|(id, _)| id.to_string())).collect()
        }
        // NVIDIA: hundreds of models; the strong ones that call tools, Nemotron Super (it also sees) first.
        "nvidia" => {
            let mut list = have(&["nvidia/nemotron-3.5-super-vl-120b-a12b", "openai/gpt-oss-120b", "moonshotai/kimi-k2-instruct-0905", "qwen/qwen3-235b-a22b", "meta/llama-3.3-70b-instruct", "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning", "openai/gpt-oss-20b"]);
            let more: Vec<String> = ids.iter().filter(|id| id.contains("nemotron") && id.contains("super") && !list.contains(id)).cloned().collect();
            list.extend(more);
            list
        }
        "deepseek" => {
            let mut list = have(&["deepseek-chat"]);
            let more: Vec<String> = ids.iter().filter(|id| !list.contains(id)).cloned().collect();
            list.extend(more);
            list
        }
        "mistral" => {
            let mut list = have(&["mistral-large-latest", "mistral-medium-latest", "mistral-small-latest"]);
            let more: Vec<String> = ids.iter().filter(|id| id.contains("large") && !list.contains(id)).cloned().collect();
            list.extend(more);
            list
        }
        _ => ids.to_vec(),
    }
}

/// A model of the same provider for her eyes (one that sees pictures), given the thinking one.
fn eyes_for(provider: &str, v: &serde_json::Value, ids: &[String], thinking: &str) -> Option<String> {
    match provider {
        "gemini" | "openai" => Some(thinking.to_string()),
        "nvidia" => (thinking.contains("-vl") || thinking.contains("vision") || thinking.contains("omni")).then(|| thinking.to_string()),
        "groq" => ids.iter().find(|id| id.contains("llama-4-maverick")).or_else(|| ids.iter().find(|id| id.contains("llama-4-scout"))).cloned(),
        "openrouter" => v["data"].as_array().into_iter().flatten().find(|m| m["id"].as_str() == Some(thinking) && m["architecture"]["input_modalities"].as_array().is_some_and(|a| a.iter().any(|x| x == "image"))).map(|_| thinking.to_string()),
        _ => None,
    }
}

/// Whether `key` works for `provider`: a cheap call that needs it (its list of models).
pub async fn check_key(provider: &str, key: &str) -> Result<()> {
    let http = http(Duration::from_secs(12));
    let r = if provider == "anthropic" {
        http.get("https://api.anthropic.com/v1/models").header("x-api-key", key).header("anthropic-version", "2023-06-01").send().await?
    } else {
        let Some(c) = COMPAT.iter().find(|c| c.prefix == provider && c.key_env.is_some()) else { bail!("unknown provider \"{provider}\"") };
        http.get(format!("{}/models", c.base)).bearer_auth(key).send().await?
    };
    if r.status().is_success() { Ok(()) } else { bail!("{} said {}", provider, r.status()) }
}

pub(crate) fn http(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder().timeout(timeout).connect_timeout(Duration::from_secs(5)).user_agent("Capricorn/0.1").build().expect("http client")
}

/// `provider/model`, or `http://host:port/v1#model` for any other OpenAI-compatible server.
pub fn model(id: &str, keys: &KeyLookup) -> Result<Arc<dyn Model>> {
    if let Some((base, name)) = id.split_once('#')
        && base.starts_with("http")
    {
        let local = base.contains("127.0.0.1") || base.contains("localhost");
        return Ok(Arc::new(OpenAiCompatible::new(id, base, name, None, local)));
    }
    let Some((provider, name)) = id.split_once('/') else { bail!("model id \"{id}\" should look like provider/model") };
    match provider {
        "ollama" => Ok(Arc::new(Ollama::new(id, name))),
        // Your own plan, through its app on this PC (no key: the app is signed in).
        "claudecode" => Ok(Arc::new(plan::PlanBrain::new(id, plan::Kind::Claude, name)?)),
        "cursor" => Ok(Arc::new(plan::PlanBrain::new(id, plan::Kind::Cursor, name)?)),
        "codex" => Ok(Arc::new(plan::PlanBrain::new(id, plan::Kind::Codex, name)?)),
        "anthropic" => match keys("anthropic") {
            Some(key) => Ok(Arc::new(Anthropic::new(id, name, key))),
            None => bail!("no Anthropic API key (her settings › Brain, or ANTHROPIC_API_KEY)"),
        },
        other => {
            let Some(c) = COMPAT.iter().find(|c| c.prefix == other) else { bail!("unknown provider \"{other}\"") };
            let key = match c.key_env {
                Some(_) => match keys(other) {
                    Some(k) => Some(k),
                    None => bail!("no {other} API key"),
                },
                None => None,
            };
            Ok(Arc::new(OpenAiCompatible::new(id, c.base, name, key, c.local)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{candidates, eyes_for};
    use serde_json::json;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_newest_stable_gemini_flash_first() {
        let list = ids(&["gemini-2.0-flash", "gemini-2.5-flash", "gemini-2.5-flash-lite", "gemini-2.5-pro", "gemini-2.5-flash-preview-tts", "gemini-3-flash-preview", "gemini-3-flash", "gemini-2.5-flash-image", "text-embedding-004"]);
        assert_eq!(candidates("gemini", &json!({}), &list)[..3], ["gemini-3-flash".to_string(), "gemini-3-flash-preview".into(), "gemini-2.5-flash".into()]);
        assert_eq!(candidates("gemini", &json!({}), &ids(&["gemini-2.5-flash-lite", "gemini-2.5-pro"])), ["gemini-2.5-flash-lite".to_string()]);
    }

    #[test]
    fn a_groq_model_that_calls_tools_and_one_that_sees() {
        let list = ids(&["llama-3.1-8b-instant", "llama-3.3-70b-versatile", "openai/gpt-oss-120b", "meta-llama/llama-4-scout-17b-16e-instruct", "whisper-large-v3"]);
        assert_eq!(candidates("groq", &json!({}), &list)[0], "openai/gpt-oss-120b");
        assert_eq!(eyes_for("groq", &json!({}), &list, "openai/gpt-oss-120b").as_deref(), Some("meta-llama/llama-4-scout-17b-16e-instruct"));
        assert!(candidates("groq", &json!({}), &ids(&["llama-3.1-8b-instant"])).is_empty());
    }

    #[test]
    fn openai_newest_gpt_for_chat() {
        let list = ids(&["gpt-4o", "gpt-5", "gpt-5-mini", "gpt-5-nano", "gpt-5.2", "gpt-5.2-mini", "gpt-realtime", "gpt-image-1", "gpt-4o-audio-preview", "o3"]);
        let c = candidates("openai", &json!({}), &list);
        assert_eq!(c[..2], ["gpt-5.2".to_string(), "gpt-5.2-mini".into()]);
        assert!(!c.iter().any(|m| m.contains("nano") || m.contains("realtime") || m.contains("image")));
    }
}
