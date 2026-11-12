//! Says a line out loud with her voice:
//!
//!     cargo run -p capri-voice --release --example say -- "Capri online. Miss me?"

use std::path::PathBuf;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let text = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let spec = capri_voice::VoiceSpec { engine: capri_voice::Engine::Supertonic, speaker: 1, speed: 1.0 };
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let t = Instant::now();
    let speaker = capri_voice::start(models, spec, move |s| match s {
        capri_voice::Said::Level { .. } => {}
        capri_voice::Said::Finished { .. } => {
            println!("{:>5} ms  {s:?}", t.elapsed().as_millis());
            let _ = done_tx.send(());
        }
        other => println!("{:>5} ms  {other:?}", t.elapsed().as_millis()),
    })?;
    println!("{:>5} ms  loaded", t.elapsed().as_millis());
    let t2 = Instant::now();
    speaker.say(&text);
    done_rx.recv()?;
    println!("spoke in {} ms after asking", t2.elapsed().as_millis());
    Ok(())
}
