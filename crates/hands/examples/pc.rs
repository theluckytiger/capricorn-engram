//! What she feels of this PC right now: `cargo run -p capri-hands --example pc`
use capri_hands::platform::system;

fn main() {
    println!("online: {:?}", system::online());
    println!("battery: {:?}", system::battery());
    println!("cpu: {:?}", system::cpu_load(std::time::Duration::from_secs(1)));
    println!("memory: {:?}", system::memory());
    println!("disks: {:?}", system::disks().iter().map(|(l, f, t)| format!("{l} {}/{} GB free", f >> 30, t >> 30)).collect::<Vec<_>>());
    println!("removable: {:?}", system::removable());
    println!("idle: {:?}", system::idle());
}
