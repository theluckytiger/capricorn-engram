//! Her ears, live, for a while: prints what they hear. With WAV files, those are played in instead of the mic, at
//! real-time speed, through the same voice detection and wake logic:
//!
//!     cargo run -p capri-ears --release --example listen -- 30
//!     cargo run -p capri-ears --release --example listen -- 20 clip1.wav clip2.wav

use std::path::PathBuf;
use std::time::Duration;

use capri_ears::engine::Source;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let files: Vec<String> = args.collect();
    let models = std::env::var("CAPRI_MODELS").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    let source = if files.is_empty() {
        Source::Mic
    } else {
        let mut all = Vec::new();
        for f in &files {
            let w = sherpa_onnx::Wave::read(f).ok_or_else(|| anyhow::anyhow!("can't read {f}"))?;
            assert_eq!(w.sample_rate(), capri_ears::mic::RATE, "{f}: 16 kHz please");
            all.extend_from_slice(w.samples());
            all.extend(std::iter::repeat_n(0.0, capri_ears::mic::RATE as usize * 3 / 2));
        }
        Source::Samples(all)
    };
    let start = std::time::Instant::now();
    let ears = capri_ears::start(capri_ears::Config { models, threads: 4, source, keep_audio: None }, move |h| match h {
        // A level line a second, not four.
        capri_ears::Heard::Level { rms } => {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed).is_multiple_of(4) {
                println!("{:5.1} s  level {:.4}", start.elapsed().as_secs_f32(), rms);
            }
        }
        other => println!("{:5.1} s  {}", start.elapsed().as_secs_f32(), serde_json::to_string(&other).unwrap()),
    })?;
    ears.set_always(true);
    std::thread::sleep(Duration::from_secs(secs));
    Ok(())
}
