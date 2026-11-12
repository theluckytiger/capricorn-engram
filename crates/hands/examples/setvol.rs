//! Sets the speakers' volume: `cargo run -p capri-hands --example setvol -- 32`.

fn main() -> anyhow::Result<()> {
    let p: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(30);
    println!("before: {:?}", capri_hands::platform::volume());
    capri_hands::platform::set_volume(p)?;
    println!("after: {:?}", capri_hands::platform::volume());
    Ok(())
}
