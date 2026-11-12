//! What UI Automation says about a control: `cargo run -p capri-hands --example inspect -- <window> <control name>`.

fn main() -> anyhow::Result<()> {
    use capri_hands::platform;
    let mut a = std::env::args().skip(1);
    let (win, name) = (a.next().unwrap_or_default(), a.next().unwrap_or_default());
    let w = capri_hands::pc::find_window(&win, &platform::list_windows()).map_err(|o| anyhow::anyhow!(o.summary))?;
    let cs = platform::controls(w.id, 1500)?;
    for c in cs.iter().filter(|c| c.name.eq_ignore_ascii_case(&name)) {
        println!("{} \"{}\": {}", c.kind, c.name, platform::inspect(w.id, c)?);
    }
    Ok(())
}
