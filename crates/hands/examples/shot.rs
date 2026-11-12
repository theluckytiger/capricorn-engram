//! Checks a screenshot without her: size, and how much of it isn't black. `cargo run -p capri-hands --example shot`
fn main() -> anyhow::Result<()> {
    let s = capri_hands::platform::screen_shot()?;
    let lit = s.rgb.chunks(3).filter(|p| p.iter().any(|&c| c > 16)).count();
    println!("{}x{} at {},{}; {:.0}% not black", s.width, s.height, s.left, s.top, 100.0 * lit as f64 / (s.width * s.height) as f64);
    Ok(())
}
