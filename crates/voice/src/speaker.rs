//! Her voice at work: lines go in, each sentence is made and queued while the one before it plays, her loudness goes
//! out (for her mouth), and `stop` cuts her off.
//!
//! The speakers are opened only while she talks and closed a few seconds after: an always-open audio stream would
//! keep Windows from going to sleep.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

use crate::engine::{self, VoiceSpec};
use crate::sentences;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Said {
    /// Her first sound for line `id` is playing. `read_as`: the line as her voice reads it (numbers and unknown
    /// words spelled out), for her journal.
    Started { id: u64, read_as: String },
    /// How loud she is right now, 0..1 (her mouth).
    Level { rms: f32 },
    /// Line `id` is over (all of it, or cut off).
    Finished { id: u64, cut: bool },
    Error { message: String },
}

enum Cmd {
    Say { id: u64, text: String },
    Stop,
    Voice(VoiceSpec),
}

#[derive(Clone)]
pub struct Speaker {
    tx: Sender<Cmd>,
    stop: Arc<AtomicBool>,
    next: Arc<AtomicU32>,
}

impl Speaker {
    /// Says a line; returns its id (in the events).
    pub fn say(&self, text: &str) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) as u64;
        self.stop.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Say { id, text: text.to_string() });
        id
    }
    /// Stops talking now, mid-word, and drops what was queued.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Stop);
    }
    pub fn set_voice(&self, spec: VoiceSpec) {
        let _ = self.tx.send(Cmd::Voice(spec));
    }
}

/// What the speaker callback plays from, shared between threads.
struct Queue {
    samples: Mutex<VecDeque<f32>>,
    /// Loudness of the last buffer played, as f32 bits.
    level: AtomicU32,
}

pub fn start(models: PathBuf, spec: VoiceSpec, on: impl Fn(Said) + Send + 'static) -> Result<Speaker> {
    let tts = engine::load(&models, spec.engine, 4)?;
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    std::thread::Builder::new().name("voice".into()).spawn(move || run(rx, models, tts, spec, s2, on))?;
    Ok(Speaker { tx, stop, next: Arc::new(AtomicU32::new(1)) })
}

struct Out {
    _stream: cpal::Stream,
    rate: i32,
    queue: Arc<Queue>,
}

fn open_output() -> Result<Out> {
    let device = cpal::default_host().default_output_device().ok_or_else(|| anyhow!("no speakers"))?;
    let config = device.default_output_config()?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate() as i32;
    let queue = Arc::new(Queue { samples: Mutex::new(VecDeque::new()), level: AtomicU32::new(0) });
    let q = queue.clone();
    let err = |e| tracing::warn!("speakers: {e}");
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config.into(),
            move |data: &mut [f32], _: &_| fill(&q, data, channels, |s| s),
            err,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream(
            config.into(),
            move |data: &mut [i16], _: &_| fill(&q, data, channels, |s| (s.clamp(-1.0, 1.0) * 32767.0) as i16),
            err,
            None,
        )?,
        other => return Err(anyhow!("speaker format {other:?} isn't supported")),
    };
    stream.play()?;
    Ok(Out { _stream: stream, rate, queue })
}

/// The speakers' callback: her queued samples (mono) on every channel, silence when there's nothing.
fn fill<T: Copy>(q: &Queue, data: &mut [T], channels: usize, from: impl Fn(f32) -> T) {
    let mut samples = q.samples.lock().unwrap();
    let mut sum = 0.0f32;
    let mut n = 0;
    for frame in data.chunks_mut(channels) {
        let s = samples.pop_front().unwrap_or(0.0);
        sum += s * s;
        n += 1;
        for out in frame {
            *out = from(s);
        }
    }
    q.level.store((sum / n.max(1) as f32).sqrt().to_bits(), Ordering::Relaxed);
}

fn run(rx: Receiver<Cmd>, models: PathBuf, mut tts: sherpa_onnx::OfflineTts, mut spec: VoiceSpec, stop: Arc<AtomicBool>, on: impl Fn(Said)) {
    let mut out: Option<Out> = None;
    // Lexicon voices skip any word they don't know: their text is normalized first.
    let mut lexicon = spec.engine.needs_lexicon().then(|| engine::lexicon(&models)).flatten();
    let mut idle_since = Instant::now();
    loop {
        let cmd = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => {
                // Close the speakers once she's been quiet a while.
                if out.as_ref().is_some_and(|o| o.queue.samples.lock().unwrap().is_empty()) && idle_since.elapsed() > Duration::from_secs(5) {
                    out = None;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match cmd {
            Cmd::Stop => {
                if let Some(o) = &out {
                    o.queue.samples.lock().unwrap().clear();
                }
            }
            Cmd::Voice(v) => {
                if v.engine != spec.engine {
                    match engine::load(&models, v.engine, 4) {
                        Ok(t) => {
                            tts = t;
                            lexicon = v.engine.needs_lexicon().then(|| engine::lexicon(&models)).flatten();
                        }
                        Err(e) => {
                            on(Said::Error { message: format!("{e:#}") });
                            continue;
                        }
                    }
                }
                spec = v;
            }
            Cmd::Say { id, text } => {
                if out.is_none() {
                    match open_output() {
                        Ok(o) => out = Some(o),
                        Err(e) => {
                            on(Said::Error { message: format!("{e:#}") });
                            on(Said::Finished { id, cut: true });
                            continue;
                        }
                    }
                }
                let o = out.as_ref().unwrap();
                // Words her voice doesn't know yet: learned, kept, and her voice reloaded with them, so even the
                // first time they're said as words.
                if let Some(l) = lexicon.as_mut() {
                    let new = crate::normalize::learn(&text, l);
                    if !new.is_empty() && engine::remember(&new).is_ok() {
                        let t = Instant::now();
                        for (w, _) in &new {
                            l.add(w);
                        }
                        match engine::load(&models, spec.engine, 4) {
                            Ok(t) => tts = t,
                            Err(e) => tracing::warn!("voice: reloading with new words: {e:#}"),
                        }
                        tracing::info!("voice learned {:?} ({} ms)", new.iter().map(|(w, s)| format!("{w}: {s}")).collect::<Vec<_>>(), t.elapsed().as_millis());
                    }
                }
                let text = match &lexicon {
                    Some(l) => crate::normalize::for_lexicon(&text, l),
                    None => text,
                };
                let cut = speak(&tts, &spec, o, id, &text, &stop, &on);
                on(Said::Finished { id, cut });
                idle_since = Instant::now();
            }
        }
    }
}

/// Makes each sentence and queues it, reporting her loudness while it plays. `true` if she was cut off.
fn speak(tts: &sherpa_onnx::OfflineTts, spec: &VoiceSpec, out: &Out, id: u64, text: &str, stop: &AtomicBool, on: &impl Fn(Said)) -> bool {
    let mut started = false;
    let resampler = (tts.sample_rate() != out.rate).then(|| sherpa_onnx::LinearResampler::create(tts.sample_rate(), out.rate)).flatten();
    let report = |out: &Out| {
        let rms = f32::from_bits(out.queue.level.load(Ordering::Relaxed));
        on(Said::Level { rms });
    };
    for sentence in sentences::split(text) {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        let (samples, _) = match engine::synth(tts, &sentence, spec.speaker, spec.speed) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("voice: {e:#}");
                continue;
            }
        };
        let samples = match &resampler {
            Some(r) => r.resample(&samples, true),
            None => samples,
        };
        out.queue.samples.lock().unwrap().extend(samples.iter().copied().chain(std::iter::repeat_n(0.0, out.rate as usize / 8)));
        if !started {
            started = true;
            on(Said::Started { id, read_as: text.to_string() });
        }
        // Make the next sentence while this one plays, but don't run more than one ahead.
        while out.queue.samples.lock().unwrap().len() > out.rate as usize / 2 {
            if stop.load(Ordering::Relaxed) {
                out.queue.samples.lock().unwrap().clear();
                return true;
            }
            report(out);
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    // Let the last sentence finish.
    while !out.queue.samples.lock().unwrap().is_empty() {
        if stop.load(Ordering::Relaxed) {
            out.queue.samples.lock().unwrap().clear();
            return true;
        }
        report(out);
        std::thread::sleep(Duration::from_millis(50));
    }
    on(Said::Level { rms: 0.0 });
    false
}
