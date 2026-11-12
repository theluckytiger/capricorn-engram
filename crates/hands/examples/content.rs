//! Files with these words in them, from Windows' search index: `cargo run -p capri-hands --example content -- budget`.

#[tokio::main]
async fn main() {
    use capri_hands::Tool;
    let words = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let o = capri_hands::files::Files.call(serde_json::json!({ "action": "search", "words": words }), &capri_hands::Ctx::default()).await;
    println!("{:?}: {}", o.status, o.summary);
}
