//! How her voice will read lines, word by word, with Kokoro's real lexicon and her own word list:
//!
//!     cargo run -p capri-voice --example read_as -- "I opened the channel's page." "That's fine, V."
//!     cargo run -p capri-voice --example read_as -- --learn lines.txt
//!
//! `--learn`: the words she doesn't know, from each line of the file, are learned into her word list (her settings
//! folder's words.txt), as she would before saying them. Models come from `models/` (CAPRI_MODELS overrides).

use std::path::PathBuf;

fn main() {
    let models = std::env::var("CAPRI_MODELS").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    let dir = directories::ProjectDirs::from("", "", "capricorn-engram").expect("a home folder").config_dir().to_path_buf();
    capri_voice::engine::set_learned(dir.join("words.txt"));
    let mut lex = capri_voice::engine::lexicon(&models).expect("Kokoro's lexicon");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (learning, lines): (bool, Vec<String>) = match args.first().map(String::as_str) {
        Some("--learn") => (true, std::fs::read_to_string(&args[1]).expect("the file").lines().map(str::to_string).collect()),
        _ => (false, args),
    };
    let mut all = 0;
    for line in lines {
        let new = capri_voice::normalize::learn(&line, &lex);
        for (w, s) in &new {
            println!("  learned {w}: {s}");
            lex.add(w);
        }
        all += new.len();
        if learning {
            capri_voice::engine::remember(&new).expect("her word list");
        } else {
            println!("{line}\n  → {}", capri_voice::normalize::for_lexicon(&line, &lex));
        }
    }
    println!("{all} new words");
}
