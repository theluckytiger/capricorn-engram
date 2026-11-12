//! Renders the same lines in each candidate voice, one WAV per voice, and how fast each one is:
//!
//!     cargo run -p capri-voice --release --example samples -- <out dir>

use std::path::PathBuf;
use std::time::Instant;

use capri_voice::engine::{self, Engine};

const LINES: &[&str] = &[
    "Capri online. Your screen's cozy. I'm keeping it.",
    "It's not raining right now, but it starts around four and stops by eleven.",
    "Opening Discord. Try not to get lost in there, okay?",
];

fn main() -> anyhow::Result<()> {
    let out = PathBuf::from(std::env::args().nth(1).expect("an output folder"));
    std::fs::create_dir_all(&out)?;
    let models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let voices: &[(Engine, &[(i32, &str)])] = &[
        (
            Engine::Kokoro,
            &[(10, "kokoro-af_sky"), (7, "kokoro-af_nova"), (2, "kokoro-af_bella"), (3, "kokoro-af_heart"), (6, "kokoro-af_nicole"), (5, "kokoro-af_kore"), (4, "kokoro-af_jessica"), (23, "kokoro-bf_lily"), (37, "kokoro-jf_alpha"), (40, "kokoro-jf_tebukuro")],
        ),
        (Engine::Kitten, &[(1, "kitten-voice-2-f"), (3, "kitten-voice-3-f"), (5, "kitten-voice-4-f"), (7, "kitten-voice-5-f")]),
        (Engine::Supertonic, &[(0, "supertonic-0"), (1, "supertonic-1"), (2, "supertonic-2"), (3, "supertonic-3"), (4, "supertonic-4"), (5, "supertonic-5"), (6, "supertonic-6"), (7, "supertonic-7"), (8, "supertonic-8"), (9, "supertonic-9")]),
    ];
    for (eng, list) in voices {
        let t = Instant::now();
        let tts = match engine::load(&models, *eng, 4) {
            Ok(t) => t,
            Err(e) => {
                println!("{eng:?}: {e:#}");
                continue;
            }
        };
        println!("{eng:?}: loaded in {} ms, {} speakers, {} Hz", t.elapsed().as_millis(), tts.num_speakers(), tts.sample_rate());
        for (sid, name) in *list {
            if *sid >= tts.num_speakers() {
                continue;
            }
            let mut all = Vec::new();
            let mut rate = 24000;
            let mut first_ms = 0;
            let t = Instant::now();
            for (i, line) in LINES.iter().enumerate() {
                let t1 = Instant::now();
                let (s, r) = engine::synth(&tts, line, *sid, 1.0)?;
                if i == 0 {
                    first_ms = t1.elapsed().as_millis();
                }
                rate = r;
                all.extend_from_slice(&s);
                all.extend(std::iter::repeat_n(0.0, r as usize / 2));
            }
            let audio_s = all.len() as f32 / rate as f32;
            let ms = t.elapsed().as_millis();
            sherpa_onnx::write(&out.join(format!("{name}.wav")).to_string_lossy(), &all, rate);
            println!("  {name:<22} first line in {first_ms:>4} ms, {audio_s:.1} s of audio in {ms} ms (x{:.1} real time)", audio_s * 1000.0 / ms as f32);
        }
    }
    Ok(())
}
