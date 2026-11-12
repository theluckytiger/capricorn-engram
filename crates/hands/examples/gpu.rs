//! What game detection sees, every 2 s for a while: the window in front, whether it fills its screen, and the top
//! users of the GPU's 3D engines.
//!
//!     cargo run -p capri-hands --example gpu -- 10

fn main() -> anyhow::Result<()> {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let mut meter = capri_hands::platform::GpuMeter::new()?;
    for _ in 0..secs / 2 {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let gpu = meter.sample()?;
        let wins = capri_hands::platform::list_windows();
        let front = wins.iter().find(|w| w.foreground);
        let mut top: Vec<(u32, f64)> = gpu.into_iter().filter(|(_, v)| *v > 0.5).collect();
        top.sort_by(|a, b| b.1.total_cmp(&a.1));
        let name = |pid: u32| capri_hands::platform::exe_path(pid).and_then(|p| std::path::Path::new(&p).file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| pid.to_string());
        println!(
            "front: {} (fullscreen {}) | GPU 3D: {}",
            front.map(|w| w.exe.as_str()).unwrap_or("-"),
            capri_hands::platform::front_fullscreen(),
            top.iter().take(5).map(|(p, v)| format!("{} {v:.0}%", name(*p))).collect::<Vec<_>>().join(", ")
        );
    }
    Ok(())
}
