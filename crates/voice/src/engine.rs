//! The speech engines she can talk with, all local, all through sherpa-onnx: Kokoro, Kitten and Supertonic.

use std::path::Path;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKittenModelConfig, OfflineTtsKokoroModelConfig, OfflineTtsModelConfig, OfflineTtsSupertonicModelConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Kokoro,
    Kitten,
    Supertonic,
}

/// Kokoro in full precision (faster on most CPUs) when it's there, else the int8 build. `CAPRI_KOKORO=int8` forces it.
pub const KOKORO: &str = "kokoro-multi-lang-v1_0";
pub const KOKORO_INT8: &str = "kokoro-int8-multi-lang-v1_0";
pub const KITTEN: &str = "kitten-mini-en-v0_8";
pub const SUPERTONIC: &str = "sherpa-onnx-supertonic-3-tts-int8-2026-05-11";

/// Her voice: an engine, a speaker in it, and how fast she talks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSpec {
    pub engine: Engine,
    pub speaker: i32,
    pub speed: f32,
}

/// Kokoro's folder and model file on this PC.
fn kokoro(models: &Path) -> Option<(std::path::PathBuf, &'static str)> {
    let int8 = (models.join(KOKORO_INT8), "model.int8.onnx");
    let full = (models.join(KOKORO), "model.onnx");
    let prefer_int8 = std::env::var("CAPRI_KOKORO").is_ok_and(|v| v == "int8");
    let order = if prefer_int8 { [int8, full] } else { [full, int8] };
    order.into_iter().find(|(d, f)| d.join(f).exists())
}

impl Engine {
    pub fn dir(self, models: &Path) -> std::path::PathBuf {
        match self {
            Engine::Kokoro => kokoro(models).map(|(d, _)| d).unwrap_or_else(|| models.join(KOKORO)),
            Engine::Kitten => models.join(KITTEN),
            Engine::Supertonic => models.join(SUPERTONIC),
        }
    }

    /// Says only words from a pronunciation lexicon (so text goes through `normalize` first).
    pub fn needs_lexicon(self) -> bool {
        matches!(self, Engine::Kokoro | Engine::Kitten)
    }
}

pub fn installed(models: &Path, engine: Engine) -> bool {
    match engine {
        Engine::Kokoro => kokoro(models).is_some(),
        other => other.dir(models).is_dir(),
    }
}

/// Her own word list (words she learned to say: `normalize::learn`), beside Kokoro's lexicons. Set once by the app.
static LEARNED: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Where her learned words are kept (her settings folder).
pub fn set_learned(path: std::path::PathBuf) {
    let _ = LEARNED.set(path);
}

/// Words she just learned, added to her word list (her voice loads them next time it's loaded).
pub fn remember(words: &[(String, String)]) -> std::io::Result<()> {
    use std::io::Write;
    let Some(path) = LEARNED.get() else { return Err(std::io::Error::other("no word list")) };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    for (w, sounds) in words {
        writeln!(f, "{w} {sounds}")?;
    }
    Ok(())
}

/// Her way of talking drops g's ("kickin'", "chompin'"), and each new one cost her voice a reload: 2 s of silence before the
/// line. All of them, worked out once from the words ending in -ing.
fn dropped_g(k: &Path) -> Option<std::path::PathBuf> {
    let file = k.join("lexicon-capri-dropped-g.txt");
    if !file.exists() {
        let us = std::fs::read_to_string(k.join("lexicon-us-en.txt")).ok()?;
        let known: std::collections::HashSet<&str> = us.lines().filter_map(|l| l.split(' ').next()).collect();
        let lines: String = us
            .lines()
            .filter_map(|l| {
                let (word, sounds) = l.split_once(' ')?;
                let stem = word.strip_suffix("ing").filter(|s| !s.is_empty())?;
                let sounds = sounds.strip_suffix("ɪ ŋ")?;
                let short = format!("{stem}in");
                (!known.contains(short.as_str())).then(|| format!("{short} {sounds}ɪ n\n"))
            })
            .collect();
        std::fs::write(&file, lines).ok()?;
    }
    Some(file)
}

/// The lexicon files: Kokoro's two, the dropped g's, and her own word list when there is one.
fn lexicon_files(k: &Path) -> Vec<std::path::PathBuf> {
    let mut files = vec![k.join("lexicon-us-en.txt"), k.join("lexicon-gb-en.txt")];
    files.extend(dropped_g(k));
    if let Some(l) = LEARNED.get().filter(|l| std::fs::metadata(l).is_ok_and(|m| m.len() > 0)) {
        files.push(l.clone());
    }
    files
}

/// The words Kokoro's lexicons know (Kitten uses them too), and hers, for `normalize::for_lexicon`.
pub fn lexicon(models: &Path) -> Option<crate::normalize::Lexicon> {
    let (dir, _) = kokoro(models)?;
    let files = lexicon_files(&dir);
    crate::normalize::Lexicon::load(&files.iter().map(|f| f.as_path()).collect::<Vec<_>>(), Some(&dir.join("tokens.txt"))).ok()
}

/// Loads an engine. Kokoro and Kitten read words through a pronunciation lexicon (sherpa-onnx 2 has no eSpeak
/// fallback); Kitten has none of its own and shares Kokoro's (same phoneme set). Supertonic reads raw text.
pub fn load(models: &Path, engine: Engine, threads: i32) -> Result<OfflineTts> {
    let dir = engine.dir(models);
    let p = |f: &str| Some(dir.join(f).to_string_lossy().into_owned());
    let kokoro_lexicon = || {
        let (k, _) = kokoro(models)?;
        Some(lexicon_files(&k).iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(","))
    };
    let mut model = OfflineTtsModelConfig { num_threads: threads, ..Default::default() };
    match engine {
        Engine::Kokoro => {
            let file = kokoro(models).map(|(_, f)| f).unwrap_or("model.onnx");
            model.kokoro = OfflineTtsKokoroModelConfig { model: p(file), voices: p("voices.bin"), tokens: p("tokens.txt"), lexicon: kokoro_lexicon(), ..Default::default() };
        }
        Engine::Kitten => {
            if !installed(models, Engine::Kokoro) {
                return Err(anyhow!("Kitten borrows Kokoro's lexicon: install Kokoro too"));
            }
            model.kitten = OfflineTtsKittenModelConfig { model: p("model.onnx"), voices: p("voices.bin"), tokens: p("tokens.txt"), lexicon: kokoro_lexicon(), ..Default::default() };
        }
        Engine::Supertonic => {
            model.supertonic = OfflineTtsSupertonicModelConfig {
                duration_predictor: p("duration_predictor.int8.onnx"),
                text_encoder: p("text_encoder.int8.onnx"),
                vector_estimator: p("vector_estimator.int8.onnx"),
                vocoder: p("vocoder.int8.onnx"),
                tts_json: p("tts.json"),
                unicode_indexer: p("unicode_indexer.bin"),
                voice_style: p("voice.bin"),
            };
        }
    }
    let config = OfflineTtsConfig { model, max_num_sentences: 1, ..Default::default() };
    OfflineTts::create(&config).ok_or_else(|| anyhow!("couldn't load {:?} from {}", engine, dir.display()))
}

/// One piece of text as audio: samples and their sample rate.
pub fn synth(tts: &OfflineTts, text: &str, speaker: i32, speed: f32) -> Result<(Vec<f32>, i32)> {
    let config = GenerationConfig { sid: speaker, speed, ..Default::default() };
    let audio = tts.generate_with_config(text, &config, None::<fn(&[f32], f32) -> bool>).ok_or_else(|| anyhow!("no audio for \"{text}\""))?;
    Ok((audio.samples().to_vec(), audio.sample_rate()))
}
