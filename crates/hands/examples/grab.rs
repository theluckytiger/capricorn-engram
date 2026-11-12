//! One window's picture, as her eyes get it (covered or on another screen, nothing moves), saved as a PNG: for
//! building eye tests and checking what she sees.
//!
//!     cargo run -p capri-hands --example grab                   # lists the windows
//!     cargo run -p capri-hands --example grab -- <words> <out.png>  # the first window whose app or title has them

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let wins = capri_hands::platform::list_windows();
    let Some(words) = args.next() else {
        for w in &wins {
            println!("{:>10}  {:<20} {:?}  {:?}{}", w.id, w.app, w.title, w.rect, if w.minimized { "  (minimized)" } else { "" });
        }
        return Ok(());
    };
    let out = args.next().unwrap_or_else(|| "window.png".into());
    let low = words.to_lowercase();
    let w = wins.iter().find(|w| format!("{} {}", w.app, w.title).to_lowercase().contains(&low)).ok_or_else(|| anyhow::anyhow!("no window with \"{words}\""))?;
    let shot = capri_hands::platform::capture(w.id)?;
    let file = std::fs::File::create(&out)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), shot.width, shot.height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&shot.rgb)?;
    println!("{} \"{}\": {}x{} → {out}", w.app, w.title, shot.width, shot.height);
    Ok(())
}
