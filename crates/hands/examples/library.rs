//! The games her libraries list: `cargo run -p capri-hands --example library -- duty`.

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    for e in capri_hands::library::entries().into_iter().filter(|e| e.name.to_lowercase().contains(&filter)) {
        println!("{:<40} {:<8} {}", e.name, e.from, e.launch);
    }
}
