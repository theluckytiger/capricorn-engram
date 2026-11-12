//! How fast one voice is, without playing anything: the same lines made with each Kokoro build (and the text as it
//! reaches the model after normalizing), saved as WAVs for listening later.
//!
//!     cargo run -p capri-voice --release --example bench -- <out dir> [speaker]

use std::path::PathBuf;
use std::time::Instant;

use capri_voice::engine::{self, Engine};

const LINES: &[&str] = &[
    "Capri online. Your screen's cozy. I'm keeping it.",
    "Volume's at 32%. Rain starts around 04:00 and stops by 11:00, 4.8 mm in all.",
    "Opening Spotify. Try not to get lost in there, okay?",
];

fn main() -> anyhow::Result<()> {
    let out = PathBuf::from(std::env::args().nth(1).expect("an output folder"));
    let speaker: i32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(37);
    std::fs::create_dir_all(&out)?;
    let models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let lex = engine::lexicon(&models).expect("Kokoro's lexicon");
    for line in LINES {
        println!("text: {}", capri_voice::normalize::for_lexicon(line, &lex));
    }
    for build in ["full", "int8"] {
        // SAFETY of set_var: single-threaded at this point, before the engine reads it.
        unsafe { std::env::set_var("CAPRI_KOKORO", build) };
        let t = Instant::now();
        let tts = engine::load(&models, Engine::Kokoro, 4)?;
        let load = t.elapsed().as_millis();
        let mut all = Vec::new();
        let mut first = 0;
        let t = Instant::now();
        for (i, line) in LINES.iter().enumerate() {
            let t1 = Instant::now();
            let (s, r) = engine::synth(&tts, &capri_voice::normalize::for_lexicon(line, &lex), speaker, 1.0)?;
            if i == 0 {
                first = t1.elapsed().as_millis();
            }
            all.extend_from_slice(&s);
            all.extend(std::iter::repeat_n(0.0, r as usize / 2));
        }
        let ms = t.elapsed().as_millis();
        let secs = all.len() as f32 / tts.sample_rate() as f32;
        sherpa_onnx::write(&out.join(format!("kokoro-{build}-{speaker}.wav")).to_string_lossy(), &all, tts.sample_rate());
        println!("{build}: loaded in {load} ms, first line in {first} ms, {secs:.1} s of audio in {ms} ms (x{:.1} real time)", secs * 1000.0 / ms as f32);
    }
    Ok(())
}
