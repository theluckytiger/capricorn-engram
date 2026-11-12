//! Writes a line in one voice to a 16 kHz WAV (the rate her ears hear at), for testing her ears with her own voice:
//!
//!     cargo run -p capri-voice --release --example render -- out.wav 37 "Want me to mute it?"
//!
//! The number is a Kokoro speaker (37 is hers; 11–20 are men's voices). `CAPRI_WORDS=…\words.txt`: with her own
//! word list (the words she's learned to say), as in the app.

use std::path::PathBuf;

use capri_voice::engine::{self, Engine};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("an output .wav");
    let sid: i32 = args.next().expect("a speaker number").parse()?;
    let text = args.collect::<Vec<_>>().join(" ");
    let models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models");
    if let Ok(words) = std::env::var("CAPRI_WORDS") {
        engine::set_learned(words.into());
    }
    let tts = engine::load(&models, Engine::Kokoro, 4)?;
    let text = match engine::lexicon(&models) {
        Some(l) => capri_voice::normalize::for_lexicon(&text, &l),
        None => text,
    };
    let (samples, rate) = engine::synth(&tts, &text, sid, 1.0)?;
    let samples = sherpa_onnx::LinearResampler::create(rate, 16_000).map(|r| r.resample(&samples, true)).unwrap_or(samples);
    anyhow::ensure!(sherpa_onnx::write(&out, &samples, 16_000), "couldn't write {out}");
    println!("{out}: {:.1} s", samples.len() as f32 / 16_000.0);
    Ok(())
}
