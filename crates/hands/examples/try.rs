//! Try one of her tools from a terminal:
//!
//!     cargo run -p capri-hands --example try -- windows '{"action":"list"}'
//!     cargo run -p capri-hands --example try -- weather '{"place":"Lyon"}'
//!     cargo run -p capri-hands --example try -- --dry open '{"name":"discord"}'

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.first().is_some_and(|a| a == "--dry");
    if dry_run {
        args.remove(0);
    }
    let hands = capri_hands::all();
    let Some(name) = args.first() else {
        for s in hands.specs() {
            println!("{}: {}", s.name, s.description);
        }
        return;
    };
    let json = args.get(1).map(String::as_str).unwrap_or("{}");
    let value: serde_json::Value = serde_json::from_str(json).expect("arguments must be JSON");
    let ctx = capri_hands::Ctx { home: std::env::var("CAPRI_HOME").ok(), own_pid: std::process::id(), dry_run, ..Default::default() };
    let t = std::time::Instant::now();
    let out = hands.call(name, value, &ctx).await;
    println!("{:?} in {} ms\n{}", out.status, t.elapsed().as_millis(), out.summary);
    // What the tool saw on the way (goes in her journal too).
    if !out.data.is_null() {
        println!("{}", serde_json::to_string_pretty(&out.data).unwrap_or_default());
    }
}
