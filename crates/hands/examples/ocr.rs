//! What Windows' text reader finds on a window: `cargo run -p capri-hands --example ocr -- steam`.

fn main() -> anyhow::Result<()> {
    use capri_hands::platform;
    let name = std::env::args().nth(1).unwrap_or_else(|| "front".into());
    let wins = platform::list_windows();
    let w = capri_hands::pc::find_window(&name, &wins).map_err(|o| anyhow::anyhow!(o.summary))?;
    let shot = platform::capture(w.id)?;
    let t = std::time::Instant::now();
    let labels = platform::ocr::read(&shot)?;
    println!("{} \"{}\" {}x{}: {} labels in {} ms", w.app, w.title, shot.width, shot.height, labels.len(), t.elapsed().as_millis());
    for l in labels.iter().take(60) {
        println!("  {:?} {}", l.rect, l.text);
    }
    Ok(())
}
