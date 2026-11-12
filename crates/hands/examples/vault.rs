//! Saves, reads and deletes a key in Credential Manager under a test name: `cargo run -p capri-hands --example vault`
fn main() -> anyhow::Result<()> {
    use capri_hands::platform::vault;
    vault::key_set("roundtrip-test", "sk-test-123")?;
    println!("read back: {:?}", vault::key_get("roundtrip-test").map(|k| k == "sk-test-123"));
    vault::key_delete("roundtrip-test");
    println!("after delete: {:?}", vault::key_get("roundtrip-test"));
    Ok(())
}
