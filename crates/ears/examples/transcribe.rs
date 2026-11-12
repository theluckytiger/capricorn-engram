//! Reads WAV files with her ears' model and says whether her name starts them:
//!
//!     cargo run -p capri-ears --release --example transcribe -- clip1.wav clip2.wav
//!
//! Models come from `models/` at the repo root (CAPRI_MODELS overrides).

use std::path::PathBuf;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let models = std::env::var("CAPRI_MODELS").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    let t = Instant::now();
    let rec = capri_ears::engine::recognizer(&models, 4)?;
    println!("model loaded in {} ms", t.elapsed().as_millis());
    for path in std::env::args().skip(1) {
        let wave = sherpa_onnx::Wave::read(&path).ok_or_else(|| anyhow::anyhow!("can't read {path}"))?;
        let samples = if wave.sample_rate() == capri_ears::mic::RATE {
            wave.samples().to_vec()
        } else {
            sherpa_onnx::LinearResampler::create(wave.sample_rate(), capri_ears::mic::RATE).unwrap().resample(wave.samples(), true)
        };
        let t = Instant::now();
        let text = capri_ears::engine::transcribe(&rec, &samples);
        let ms = t.elapsed().as_millis();
        let secs = samples.len() as f32 / capri_ears::mic::RATE as f32;
        match capri_ears::wake::find(&text) {
            Some(rest) => println!("{path}: {secs:.1} s read in {ms} ms: \"{text}\"  → her name, then \"{rest}\""),
            // As the next sentence while she's listening: a request, or a stray sound.
            None if capri_ears::wake::meaningful(&text) => println!("{path}: {secs:.1} s read in {ms} ms: \"{text}\"  → no name (a request, if she was listening)"),
            None => println!("{path}: {secs:.1} s read in {ms} ms: \"{text}\"  → no name (a stray sound)"),
        }
    }
    Ok(())
}
