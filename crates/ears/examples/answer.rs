//! A yes said to one of her questions, end to end through her ears, on a recording played in at real-time speed:
//!
//!     cargo run -p capri-ears --release --example answer -- <clip.wav>
//!
//! The clip (16 kHz; her ears keep the last 60 in `logs\audio` in dev builds) is played twice: with no question up
//! (a lone "Okay." isn't for her), and while she's waiting for an answer (it is).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use capri_ears::engine::{Config, Heard, Source};

fn run(clip: &[f32], answering: bool) -> Vec<String> {
    let models = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let mut samples = vec![0.0f32; 16_000];
    samples.extend_from_slice(clip);
    samples.extend(vec![0.0f32; 16_000 * 2]);
    let secs = samples.len() as u64 / 16_000 + 1;
    let (tx, rx) = mpsc::channel();
    let ears = capri_ears::engine::start(Config { models, threads: 2, source: Source::Samples(samples), keep_audio: None }, move |h| {
        let _ = tx.send(h);
    })
    .expect("her ears");
    ears.set_always(true);
    if answering {
        ears.answering(Duration::from_secs(60));
    }
    let end = Instant::now() + Duration::from_secs(secs + 3);
    let mut out = Vec::new();
    while Instant::now() < end {
        if let Ok(h) = rx.recv_timeout(Duration::from_millis(100)) {
            match h {
                Heard::Level { .. } | Heard::Voice { .. } | Heard::Mic { .. } => {}
                other => out.push(format!("{other:?}")),
            }
        }
    }
    out
}

fn main() {
    let path = std::env::args().nth(1).expect("a 16 kHz WAV");
    let w = sherpa_onnx::Wave::read(&path).expect("can't read it");
    assert_eq!(w.sample_rate(), 16_000);
    let clip = w.samples().to_vec();
    println!("no question up:");
    for e in run(&clip, false) {
        println!("  {e}");
    }
    println!("waiting for an answer:");
    for e in run(&clip, true) {
        println!("  {e}");
    }
}
