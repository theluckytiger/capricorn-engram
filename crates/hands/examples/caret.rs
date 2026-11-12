//! Where the text cursor is in the window in front, ten times a second for a few seconds:
//! `cargo run -p capri-hands --example caret`
fn main() {
    for _ in 0..30 {
        let fg = capri_hands::platform::foreground();
        let app = capri_hands::platform::list_windows().into_iter().find(|w| w.id == fg).map(|w| w.app).unwrap_or_default();
        println!("{app}: {:?}", capri_hands::platform::caret());
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
