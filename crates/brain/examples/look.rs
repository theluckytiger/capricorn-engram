//! Her eyes, checked against the truth: a window's picture goes to the `eyes` model, which says where something is;
//! when UI Automation knows that control, its real place says whether the model was right. Nothing is clicked.
//!
//!     cargo run -p capri-brain --example look -- brave "the address bar" "Address and search bar"
//!     cargo run -p capri-brain --example look -- discord "the mute button" Mute
//!
//! CAPRI_EYES picks the model (default: her local one).

use std::time::Instant;

use capri_hands::eyes::find;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (window, what) = (args.first().expect("a window"), args.get(1).expect("what to find"));
    let truth = args.get(2);
    let id = std::env::var("CAPRI_EYES").unwrap_or_else(|_| "ollama/qwen3-vl:8b-instruct".into());
    let model = capri_brain::providers::model(&id, &capri_brain::providers::env_keys())?;
    let eyes = capri_brain::eyes::ModelEyes { model, gaming: Default::default() };

    let wins = capri_hands::platform::list_windows();
    let w = capri_hands::pc::find_window(window, &wins).map_err(|o| anyhow::anyhow!(o.summary))?;
    let shot = capri_hands::platform::capture(w.id)?;
    println!("{} \"{}\": {}x{} at ({}, {})", w.app, w.title, shot.width, shot.height, shot.left, shot.top);

    let t = Instant::now();
    let found = match find(&eyes, &shot.rgb, shot.width, shot.height, what).await {
        Ok(f) => f,
        Err(why) => {
            println!("{} ms: {why}", t.elapsed().as_millis());
            return Ok(());
        }
    };
    for r in &found.replies {
        println!("  {}", r.trim());
    }
    let (x, y) = (shot.left + found.x, shot.top + found.y);
    println!("{} ms → ({x}, {y}) on screen, \"{}\"", t.elapsed().as_millis(), found.label);
    if let Some(name) = truth {
        let cs = capri_hands::platform::controls(w.id, 1500)?;
        match cs.iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
            Some(c) => {
                let inside = x >= c.rect[0] && x <= c.rect[2] && y >= c.rect[1] && y <= c.rect[3];
                let (cx, cy) = c.center();
                println!("truth: {} \"{}\" at {:?} (centre {cx}, {cy}): {}", c.kind, c.name, c.rect, if inside { "HIT" } else { "MISS" });
            }
            None => println!("UI Automation has no control called \"{name}\""),
        }
    }
    Ok(())
}
