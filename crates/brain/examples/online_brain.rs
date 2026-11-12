//! Which models her online brain would use with the key saved in her settings (the key itself never shown):
//! `cargo run -p capri-brain --example online_brain -- gemini`
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = std::env::args().nth(1).unwrap_or_else(|| "gemini".into());
    #[cfg(windows)]
    let key = capri_hands::platform::vault::key_get(&provider);
    #[cfg(not(windows))]
    let key: Option<String> = None;
    // Your own plan needs no key.
    let key = if matches!(provider.as_str(), "claudecode" | "cursor" | "ollamacloud") { Some(String::new()) } else { key };
    let key = key.or_else(|| capri_brain::providers::key_env(&provider).and_then(|v| std::env::var(v).ok())).ok_or_else(|| anyhow::anyhow!("no {provider} key saved"))?;
    match capri_brain::providers::online_brain(&provider, &key, None).await {
        Ok((fast, eyes)) => println!("thinking: {fast}\neyes: {eyes:?}"),
        Err(e) => println!("failed: {e:#}"),
    }
    Ok(())
}
