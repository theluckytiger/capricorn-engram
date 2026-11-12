//! Whether spots on a window are its own title-bar buttons: `cargo run -p capri-hands --example caption -- claude`.

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "claude".into());
    let wins = capri_hands::platform::list_windows();
    let Some(w) = wins.iter().find(|w| format!("{} {}", w.app, w.title).to_lowercase().contains(&name.to_lowercase()) && !w.minimized) else { return println!("no window \"{name}\"") };
    let [l, t, r, b] = w.rect;
    for (what, x, y) in [("top-right corner", r - 20, t + 15), ("next to it", r - 70, t + 15), ("title, middle", (l + r) / 2, t + 15), ("page, middle", (l + r) / 2, (t + b) / 2), ("page, top right", r - 30, t + 200)] {
        println!("{:<18} ({x}, {y}): {}", what, capri_hands::platform::caption_button_at(w.id, x, y));
    }
}
