//! What Gemini's OpenAI-compatible API says, raw: the models the saved key can use, and one small request with a
//! tool (the key itself never shown): `cargo run -p capri-brain --example gemini_probe -- [model]`
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    let key = capri_hands::platform::vault::key_get("gemini");
    #[cfg(not(windows))]
    let key: Option<String> = None;
    let key = key.or_else(|| std::env::var("GEMINI_API_KEY").ok()).ok_or_else(|| anyhow::anyhow!("no gemini key saved"))?;
    let base = "https://generativelanguage.googleapis.com/v1beta/openai";
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(90)).build()?;
    let v: serde_json::Value = http.get(format!("{base}/models")).bearer_auth(&key).send().await?.json().await?;
    let ids: Vec<&str> = v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).filter(|id| id.contains("flash") || id.contains("pro")).collect();
    let _ = ids;
    let models: Vec<String> = std::env::args().skip(1).collect();
    let models = if models.is_empty() { vec!["gemini-3.8-flash".to_string()] } else { models };
    for spec in &models {
        let (model, effort) = match spec.split_once('@') {
            Some((m, e)) => (m.to_string(), Some(e.to_string())),
            None => (spec.clone(), None),
        };
        let mut body = serde_json::json!({
            "model": model,
            "messages": [{ "role": "system", "content": "You operate a PC with tools." }, { "role": "user", "content": "Open Discord." }],
            "max_tokens": 300,
            "tools": [{ "type": "function", "function": { "name": "open", "description": "Open an app by name", "parameters": { "type": "object", "properties": { "name": { "type": "string" }, "n": { "type": "integer", "minimum": 1 } }, "required": ["name"] } } }]
        });
        if let Some(e) = &effort {
            body["reasoning_effort"] = e.as_str().into();
        }
        let t = std::time::Instant::now();
        let r = http.post(format!("{base}/chat/completions")).bearer_auth(&key).json(&body).send().await;
        match r {
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                println!("\n{model} effort {effort:?}: {status} in {:.1} s\n{}", t.elapsed().as_secs_f32(), text.chars().take(700).collect::<String>());
            }
            Err(e) => println!("\n{model} effort {effort:?}: {e} after {:.1} s", t.elapsed().as_secs_f32()),
        }
    }
    Ok(())
}
