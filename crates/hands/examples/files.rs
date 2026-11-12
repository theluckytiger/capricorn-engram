//! Finds files by name the way she does: `cargo run -p capri-hands --example files -- <words> [kind]`
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let t = std::time::Instant::now();
    let found = capri_hands::files::find(args.first().map(String::as_str).unwrap_or(""), None, std::time::Duration::from_millis(2500));
    println!("{} found in {} ms", found.len(), t.elapsed().as_millis());
    for (p, _) in found.iter().take(5) {
        println!("  {}", p.file_name().unwrap().to_string_lossy());
    }
}
