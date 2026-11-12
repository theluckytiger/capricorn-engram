//! Ask her something from a terminal, end to end (fast path, agent, persona), with real tools:
//!
//!     cargo run -p capri-brain --example ask -- "what's playing?"
//!     cargo run -p capri-brain --example ask -- --dry "open discord and minimize spotify"
//!
//! CAPRI_FAST / CAPRI_SMART pick models (default ollama/qwen3-vl:8b-instruct, no smart); CAPRI_HOME sets home.

use std::sync::Arc;

use capri_brain::{Brain, DenyRisky, Events, StepEvent, providers};

struct Print;
impl Events for Print {
    fn step(&self, ev: &StepEvent) {
        match ev {
            StepEvent::Thinking { model } => eprintln!("  … {model}"),
            StepEvent::Plan { text } => eprintln!("  plan: {text}"),
            StepEvent::Started { tool, args } => eprintln!("  → {tool} {args}"),
            StepEvent::Finished { status, summary, .. } => eprintln!("    {status:?}: {}", summary.chars().take(160).collect::<String>()),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.first().is_some_and(|a| a == "--dry");
    if dry_run {
        args.remove(0);
    }
    let text = args.join(" ");
    let keys = providers::env_keys();
    let fast = providers::model(&std::env::var("CAPRI_FAST").unwrap_or_else(|_| "ollama/qwen3-vl:8b-instruct".into()), &keys)?;
    let smart = std::env::var("CAPRI_SMART").ok().map(|id| providers::model(&id, &keys)).transpose()?;
    // CAPRI_SCREENS=1: as if her Privacy setting let pictures of the screen go to a cloud brain (look, without asking).
    let ctx = capri_hands::Ctx { home: std::env::var("CAPRI_HOME").ok(), own_pid: std::process::id(), dry_run, screens_to_cloud: std::env::var("CAPRI_SCREENS").is_ok(), ..Default::default() };
    let brain = Brain::new(capri_hands::all(), ctx, fast, smart.map(|s: Arc<_>| s), true);
    let a = brain.ask(&text, &DenyRisky, &Print).await;
    println!("[{} {} {} ms] {}", a.via, a.model.as_deref().unwrap_or("-"), a.ms, a.line);
    Ok(())
}
