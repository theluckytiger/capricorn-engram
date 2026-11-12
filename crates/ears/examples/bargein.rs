//! Her name said over her voice, end to end through her ears (VAD, Parakeet, the quick looks while she talks), on
//! recorded audio played in at real-time speed:
//!
//!     cargo run -p capri-ears --release --example bargein -- <folder>
//!
//! <folder> holds 16 kHz WAVs. Her lines (make them with capri-voice's `render` example, texts below): her.wav,
//! her_question.wav, her_name.wav. Recordings of you (her ears keep the last 60 in dev builds): capri.wav ("Capri."),
//! request.wav ("Pause the media."), one_breath.wav ("Capri, on mute."), unfinished.wav ("Select the text, remove it
//! and type"), rest.wav ("Google."), answer.wav ("Louder.").
//!
//! Her voice goes in the way a mic hears it off speakers (quieter, a little late, duller) and stops when her ears
//! say her name cut her off, the way the app stops her.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use capri_ears::engine::{Config, Heard, LISTEN_FOR, Source};

const RATE: usize = 16_000;
const HER: &str = "Rain's starting at four tomorrow and it stays wet all day, sixty millimeters in total, so keep the umbrella close.";
const HER_QUESTION: &str = "You're not muted. Your mic is on and you're not deafened. Want me to mute it?";
const HER_NAME: &str = "I'm what's left of Project Capricorn, a ghost in your machine, and I like it here.";

fn wav(dir: &Path, name: &str) -> Vec<f32> {
    let w = sherpa_onnx::Wave::read(&dir.join(name).to_string_lossy()).unwrap_or_else(|| panic!("{name} is missing"));
    assert_eq!(w.sample_rate(), RATE as i32, "{name} isn't 16 kHz");
    w.samples().to_vec()
}

/// Her voice as a mic hears it off speakers: quieter, 40 ms late, the highs rolled off.
fn echo(her: &[f32], gain: f32) -> Vec<f32> {
    let mut out = vec![0.0; RATE * 40 / 1000];
    let mut y = 0.0;
    for &x in her {
        y += 0.35 * (x - y);
        out.push(y * gain);
    }
    out
}

fn secs(s: &[f32]) -> f32 {
    s.len() as f32 / RATE as f32
}

type Events = Vec<(f32, Heard)>;

struct Scene {
    name: &'static str,
    /// Her line as the mic hears it, its text, and whether it's a question (her ears listen for the answer after).
    her: Option<(Vec<f32>, &'static str, bool)>,
    /// Your sounds, and when (seconds).
    you: Vec<(f32, Vec<f32>)>,
    secs: f32,
    check: fn(&Events) -> Result<(), String>,
}

fn has(ev: &Events, f: impl Fn(&Heard) -> bool) -> bool {
    ev.iter().any(|(_, h)| f(h))
}

fn request_with(ev: &Events, words: &[&str]) -> bool {
    has(ev, |h| matches!(h, Heard::Request { text, .. } if words.iter().all(|w| text.to_lowercase().contains(w))))
}

fn barged(ev: &Events) -> bool {
    has(ev, |h| matches!(h, Heard::BargeIn { .. }))
}

fn nothing_for_her(ev: &Events) -> Result<(), String> {
    if barged(ev) || has(ev, |h| matches!(h, Heard::Request { .. } | Heard::Wake)) { Err("she took her own voice for you".into()) } else { Ok(()) }
}

fn play(models: &Path, scene: &Scene) -> anyhow::Result<Events> {
    let n = (scene.secs * RATE as f32) as usize;
    let mut yours = vec![0.0f32; n];
    for (at, s) in &scene.you {
        let i = (at * RATE as f32) as usize;
        for (k, x) in s.iter().enumerate() {
            if let Some(y) = yours.get_mut(i + k) {
                *y += x;
            }
        }
    }
    let her = Arc::new(scene.her.as_ref().map(|h| h.0.clone()).unwrap_or_default());
    let yours = Arc::new(yours);
    let events: Arc<Mutex<Events>> = Arc::default();
    let start: Arc<Mutex<Option<Instant>>> = Arc::default();
    let cut = Arc::new(AtomicBool::new(false));
    let her_over = Arc::new(AtomicBool::new(her.is_empty()));

    let feed = {
        let (her, yours, start, cut, her_over) = (her.clone(), yours.clone(), start.clone(), cut.clone(), her_over.clone());
        move || {
            let (tx, rx) = mpsc::channel();
            let (her, yours, start, cut, her_over) = (her.clone(), yours.clone(), start.clone(), cut.clone(), her_over.clone());
            std::thread::spawn(move || {
                let t0 = Instant::now();
                *start.lock().unwrap() = Some(t0);
                let chunk = RATE / 16;
                for c in 0.. {
                    let mut buf = vec![0.0f32; chunk];
                    for (k, b) in buf.iter_mut().enumerate() {
                        let i = c * chunk + k;
                        *b = yours.get(i).copied().unwrap_or(0.0);
                        if i < her.len() && !cut.load(Ordering::Relaxed) {
                            *b += her[i];
                        } else if i >= her.len() {
                            her_over.store(true, Ordering::Relaxed);
                        }
                    }
                    if tx.send(buf).is_err() {
                        return;
                    }
                    // Real-time speed, like a mic.
                    let due = t0 + Duration::from_micros(62_500 * (c as u64 + 1));
                    std::thread::sleep(due.saturating_duration_since(Instant::now()));
                }
            });
            rx
        }
    };

    let on = {
        let (events, start, cut) = (events.clone(), start.clone(), cut.clone());
        move |h: Heard| {
            if matches!(h, Heard::Level { .. } | Heard::Voice { .. } | Heard::Mic { .. }) {
                return;
            }
            if matches!(h, Heard::BargeIn { .. }) {
                cut.store(true, Ordering::Relaxed);
            }
            let t = start.lock().unwrap().map_or(0.0, |s| s.elapsed().as_secs_f32());
            events.lock().unwrap().push((t, h));
        }
    };
    let config = Config { models: models.to_path_buf(), threads: 4, source: Source::Feed(Arc::new(feed)), keep_audio: None };
    let ears = capri_ears::start(config, on)?;
    ears.set_always(true);
    if let Some((_, text, _)) = &scene.her {
        ears.speaking(Some(text));
    }
    // The app's side: her line ends (or her name cut it off), and after a question she listens for the answer.
    let mut told = scene.her.is_none();
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f32() < scene.secs + 1.5 {
        if !told && (her_over.load(Ordering::Relaxed) || cut.load(Ordering::Relaxed)) {
            told = true;
            ears.speaking(None);
            if scene.her.as_ref().is_some_and(|h| h.2) && !cut.load(Ordering::Relaxed) {
                ears.attend(LISTEN_FOR);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(ears);
    Ok(events.lock().unwrap().clone())
}

fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(std::env::args().nth(1).expect("a folder of WAVs"));
    let models = std::env::var("CAPRI_MODELS").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    let w = |n: &str| wav(&dir, n);
    // How loud her voice is in the mic, off speakers (0.45: as loud as you; ECHO overrides).
    let gain: f32 = std::env::var("ECHO").ok().and_then(|g| g.parse().ok()).unwrap_or(0.45);
    let (her, her_q, her_name) = (echo(&w("her.wav"), gain), echo(&w("her_question.wav"), gain), echo(&w("her_name.wav"), gain));
    let (capri, request, one_breath, unfinished, rest, answer) = (w("capri.wav"), w("request.wav"), w("one_breath.wav"), w("unfinished.wav"), w("rest.wav"), w("answer.wav"));
    let after = |at: f32, s: &[f32], gap: f32| at + secs(s) + gap;
    let q_end = secs(&her_q);

    let scenes = vec![
        Scene { name: "her own voice, nothing else", her: Some((her.clone(), HER, false)), you: vec![], secs: secs(&her) + 1.5, check: nothing_for_her },
        Scene { name: "her own line says Capricorn", her: Some((her_name.clone(), HER_NAME, false)), you: vec![], secs: secs(&her_name) + 1.5, check: nothing_for_her },
        Scene {
            name: "\"Capri\" over her",
            her: Some((her.clone(), HER, false)),
            you: vec![(1.5, capri.clone())],
            secs: secs(&her) + 1.0,
            check: |ev| if barged(ev) { Ok(()) } else { Err("not cut off".into()) },
        },
        Scene {
            name: "\"Capri\" over her, pause, \"Pause the media\"",
            her: Some((her.clone(), HER, false)),
            you: vec![(1.5, capri.clone()), (after(1.5, &capri, 0.8), request.clone())],
            secs: 9.0,
            check: |ev| if !barged(ev) { Err("not cut off".into()) } else if !request_with(ev, &["pause"]) { Err("no request".into()) } else { Ok(()) },
        },
        Scene {
            name: "\"Capri, on mute\" over her, one breath",
            her: Some((her.clone(), HER, false)),
            you: vec![(1.5, one_breath.clone())],
            secs: 7.0,
            check: |ev| if !barged(ev) { Err("not cut off".into()) } else if !request_with(ev, &["mute"]) { Err("no request".into()) } else { Ok(()) },
        },
        Scene {
            name: "\"Capri\" while she talks, headphones (no echo)",
            her: Some((vec![0.0; her.len()], HER, false)),
            you: vec![(1.5, capri.clone())],
            secs: secs(&her) + 1.0,
            check: |ev| if barged(ev) { Ok(()) } else { Err("not cut off".into()) },
        },
        Scene {
            name: "her question, then your answer right after",
            her: Some((her_q.clone(), HER_QUESTION, true)),
            you: vec![(q_end + 0.25, answer.clone())],
            secs: q_end + 6.0,
            check: |ev| {
                if !request_with(ev, &["louder"]) {
                    Err("didn't take the answer".into())
                } else if request_with(ev, &["muted"]) {
                    Err("took her own words".into())
                } else {
                    Ok(())
                }
            },
        },
        Scene {
            name: "\"Capri\", then a request that stops mid-thought",
            her: None,
            you: vec![(0.5, capri.clone()), (after(0.5, &capri, 0.8), unfinished.clone()), (after(after(0.5, &capri, 0.8), &unfinished, 0.7), rest.clone())],
            secs: 12.0,
            check: |ev| {
                let n = ev.iter().filter(|(_, h)| matches!(h, Heard::Request { .. })).count();
                if n != 1 { Err(format!("{n} requests")) } else if !request_with(ev, &["type", "google"]) { Err("the halves weren't joined".into()) } else { Ok(()) }
            },
        },
        Scene {
            name: "\"Capri\", then \"Pause the media\"",
            her: None,
            you: vec![(0.5, capri.clone()), (after(0.5, &capri, 0.8), request.clone())],
            secs: 7.0,
            check: |ev| if has(ev, |h| matches!(h, Heard::Wake)) && request_with(ev, &["pause"]) { Ok(()) } else { Err("no wake and request".into()) },
        },
    ];

    let mut failed = 0;
    for s in &scenes {
        let ev = play(&models, s)?;
        let verdict = (s.check)(&ev);
        println!("\n{} {}", if verdict.is_ok() { "PASS" } else { "FAIL" }, s.name);
        for (at, gap) in s.you.iter().map(|(at, x)| (*at, secs(x))) {
            println!("    you: {at:.2}–{:.2} s", at + gap);
        }
        for (t, h) in &ev {
            println!("    {t:5.2} s  {h:?}");
        }
        if let Err(e) = verdict {
            failed += 1;
            println!("    → {e}");
        }
    }
    println!("\n{} of {} passed", scenes.len() - failed, scenes.len());
    Ok(())
}
