//! Her ears at work: the mic cut into sentences by Silero VAD, each one read by Parakeet, and a little state
//! machine on top:
//!
//! - **waiting**: every sentence is checked for her name at its start. "Capri, mute my mic" is a request at once;
//!   "Capri" alone opens her attention.
//! - **attending**: the next sentence is the request. She waits [`LISTEN_FOR`] for it to start, listens as long as it
//!   takes to say, and gives a sentence that stopped mid-thought ("…and type") a moment more for the rest.
//! - **holding**: push-to-talk; everything until release is one request.
//!
//! Conversation mode: after her spoken answer she attends again for a while ([`Ears::converse`]), so you can go on
//! without her name; what your speakers were playing then (a video, someone in a call) still needs her name.
//!
//! While she talks, her ears stay open for one thing: her name. The mic hears her too (on speakers), so a sentence
//! that started while she talked is hers unless it has her name in it, and a few times a second the last moment of
//! sound is read for her name: "Capri" said over her cuts her off within a fraction of a second, not after her line.
//!
//! Nothing is recorded or kept: audio lives in memory until it's been read (dev builds keep the last 60 sentences,
//! 3000 when she's being tested).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde::Serialize;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};

use crate::{mic, wake};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Heard {
    /// The mic opened (its name) or closed.
    Mic { open: bool, name: String },
    /// How loud the mic is, 0..1, a few times a second while it's open (her mic meter).
    Level { rms: f32 },
    /// Someone's talking (for the mic light).
    Voice { on: bool },
    /// Every sentence as read, and what she made of it (`verdict`: "request", "her name", "her own voice"…), for her
    /// journal. `audio`: where its recording was kept, when recordings are on (dev).
    Utterance { text: String, ms: u128, verdict: &'static str, audio: Option<String> },
    /// Her name, alone: she's listening for the request now.
    Wake,
    /// Her name said over her while she talks (`text`: the moment of sound it was heard in). The app stops her voice;
    /// she's listening for the request now.
    BargeIn { text: String, ms: u128 },
    /// A request, in words. `after_name`: it came in the same breath as her name.
    Request { text: String, after_name: bool, ms: u128 },
    /// She was listening and nothing came.
    Timeout,
    Error { message: String },
}

/// After her name (or a question of hers), how long she waits for you to start talking.
pub const LISTEN_FOR: Duration = Duration::from_secs(8);
/// A sentence that started in time gets this much longer to end (a video or a call can keep "voice" on forever).
const MORE: Duration = Duration::from_secs(10);
/// A request that stopped mid-thought ("select the text, remove it and type") gets this long for the rest.
const REST_OF_IT: Duration = Duration::from_millis(1200);
/// While she talks: how often the last moment of sound is read for her name, and how long that moment is.
const PEEK_EVERY: Duration = Duration::from_millis(300);
const PEEK: usize = mic::RATE as usize * 16 / 10;
/// After her voice stops, the mic still hears it for a moment (the room, the audio pipeline), in samples.
const ECHO_TAIL: i64 = mic::RATE as i64 * 35 / 100;
/// The longest request, in samples (the VAD cuts at 20 s anyway).
const LONGEST: usize = mic::RATE as usize * 20;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Waiting,
    Attending(Instant),
    Holding,
}

enum Cmd {
    Always(bool),
    Attend(Duration),
    Converse(Duration),
    Hold(bool),
    Speaking(Option<String>),
    StopListening,
    Answering(Duration),
}

/// Talk to her ears from any thread.
#[derive(Clone)]
pub struct Ears {
    tx: Sender<Cmd>,
}

impl Ears {
    /// Always listening for her name, or only when asked (`attend`, push-to-talk).
    pub fn set_always(&self, on: bool) {
        let _ = self.tx.send(Cmd::Always(on));
    }
    /// Listen for one request now (the talk key tapped, a question of hers).
    pub fn attend(&self, for_: Duration) {
        let _ = self.tx.send(Cmd::Attend(for_));
    }
    /// She's asked something and is waiting for a yes or a no: for `for_` one is taken however it's said (zero: it's
    /// been answered).
    pub fn answering(&self, for_: Duration) {
        let _ = self.tx.send(Cmd::Answering(for_));
    }
    /// Conversation: after her answer, the next sentence is a request without her name, if it starts within `for_`
    /// and isn't your speakers talking.
    pub fn converse(&self, for_: Duration) {
        let _ = self.tx.send(Cmd::Converse(for_));
    }
    /// Push-to-talk: down, then up.
    pub fn hold(&self, down: bool) {
        let _ = self.tx.send(Cmd::Hold(down));
    }
    /// She started saying `line` (`Some`), or stopped (`None`). While she talks, only her name counts.
    pub fn speaking(&self, line: Option<&str>) {
        let _ = self.tx.send(Cmd::Speaking(line.map(str::to_string)));
    }
    /// Stop listening for a request (her panel was closed).
    pub fn stop_listening(&self) {
        let _ = self.tx.send(Cmd::StopListening);
    }
}

pub struct Config {
    pub models: PathBuf,
    pub threads: i32,
    /// Where sound comes from: the mic, or (for tests) recorded audio played in at real-time speed.
    pub source: Source,
    /// Keep a recording of each sentence she reads here (the last 60), for finding out why she misheard. Dev only.
    pub keep_audio: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub enum Source {
    #[default]
    Mic,
    Samples(Vec<f32>),
    /// Tests: chunks from whatever this returns, called each time the mic would open.
    Feed(std::sync::Arc<dyn Fn() -> Receiver<Vec<f32>> + Send + Sync>),
}

/// What the engine reads chunks from; dropping it closes the source.
type Opened = (Box<dyn std::any::Any>, String, Receiver<Vec<f32>>);

fn open(source: &Source) -> Result<Opened> {
    match source {
        Source::Mic => {
            let (m, rx) = mic::open()?;
            let name = m.name.clone();
            Ok((Box::new(m), name, rx))
        }
        Source::Samples(samples) => {
            let (tx, rx) = mpsc::channel();
            let samples = samples.clone();
            std::thread::spawn(move || {
                let chunk = mic::RATE as usize / 16;
                // The recording once, then silence until the source is closed (like a mic in a quiet room).
                for c in samples.chunks(chunk).map(<[f32]>::to_vec).chain(std::iter::repeat(vec![0.0; chunk])) {
                    if tx.send(c).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(62));
                }
            });
            Ok((Box::new(()), "recording".into(), rx))
        }
        Source::Feed(f) => Ok((Box::new(()), "feed".into(), f())),
    }
}

pub fn models_ok(models: &Path) -> bool {
    models.join("silero_vad.onnx").exists() && models.join(PARAKEET).join("encoder.int8.onnx").exists()
}

const PARAKEET: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";

pub fn recognizer(models: &Path, threads: i32) -> Result<OfflineRecognizer> {
    let dir = models.join(PARAKEET);
    let p = |f: &str| Some(dir.join(f).to_string_lossy().into_owned());
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.transducer = OfflineTransducerModelConfig { encoder: p("encoder.int8.onnx"), decoder: p("decoder.int8.onnx"), joiner: p("joiner.int8.onnx") };
    config.model_config.tokens = p("tokens.txt");
    config.model_config.model_type = Some("nemo_transducer".into());
    config.model_config.num_threads = threads;
    OfflineRecognizer::create(&config).ok_or_else(|| anyhow!("couldn't load Parakeet from {}", dir.display()))
}

fn vad(models: &Path) -> Result<VoiceActivityDetector> {
    let config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: Some(models.join("silero_vad.onnx").to_string_lossy().into_owned()),
            threshold: 0.5,
            // The pause that ends a sentence: long enough for "Capri… open Steam", short enough to feel quick.
            min_silence_duration: 0.4,
            min_speech_duration: 0.2,
            window_size: 512,
            max_speech_duration: 20.0,
        },
        sample_rate: mic::RATE,
        num_threads: 1,
        ..Default::default()
    };
    VoiceActivityDetector::create(&config, 30.0).ok_or_else(|| anyhow!("couldn't load Silero VAD"))
}

/// Saves a sentence as a WAV in `dir` and keeps only the newest 60 there. Returns its path.
fn save(dir: &Path, samples: &[f32]) -> Option<String> {
    std::fs::create_dir_all(dir).ok()?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    let path = dir.join(format!("heard-{ms}.wav"));
    if !sherpa_onnx::write(&path.to_string_lossy(), samples, mic::RATE) {
        return None;
    }
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "wav")).collect();
    old.sort();
    let extra = old.len().saturating_sub(if capri_core::journal::everything() { 3000 } else { 60 });
    for p in &old[..extra] {
        let _ = std::fs::remove_file(p);
    }
    Some(path.to_string_lossy().into_owned())
}

const HISTORY: usize = mic::RATE as usize * 20;
const PREROLL: i64 = mic::RATE as i64 * 35 / 100;

/// Samples `from..to` (counted since the VAD began) out of `history` (the last samples fed, ending at `fed`).
/// `None` if they aren't all there.
fn slice(history: &[f32], fed: i64, from: i64, to: i64) -> Option<Vec<f32>> {
    let base = fed - history.len() as i64;
    (from >= base && to <= fed && from < to).then(|| history[(from - base) as usize..(to - base) as usize].to_vec())
}

/// The segment from `start` for `n` samples, with `PREROLL` before it (as much of it as the history still has).
fn with_preroll(history: &[f32], fed: i64, start: i64, n: i64) -> Option<Vec<f32>> {
    let base = fed - history.len() as i64;
    if start < base {
        return None;
    }
    slice(history, fed, (start - PREROLL).max(base), start + n)
}

/// Reads one utterance.
pub fn transcribe(rec: &OfflineRecognizer, samples: &[f32]) -> String {
    let stream = rec.create_stream();
    stream.accept_waveform(mic::RATE, samples);
    rec.decode(&stream);
    stream.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
}

/// Starts her ears (mic, VAD, Parakeet) on their own thread. `on` gets everything she hears.
pub fn start(config: Config, on: impl Fn(Heard) + Send + 'static) -> Result<Ears> {
    let (tx, rx) = mpsc::channel();
    let rec = recognizer(&config.models, config.threads)?;
    let vad = vad(&config.models)?;
    let source = config.source;
    let keep = config.keep_audio;
    std::thread::Builder::new().name("ears".into()).spawn(move || run(rx, State::new(rec, vad, keep, on), source))?;
    Ok(Ears { tx })
}

/// The line she's saying now.
struct Her {
    /// Name-like words in it: the mic hearing those while she talks is her, not someone calling her.
    names: Vec<String>,
    since: Instant,
}

/// A request that stopped mid-thought, waiting for the rest.
struct Pending {
    samples: Vec<f32>,
    text: String,
    after_name: bool,
    until: Instant,
}

struct State<F: Fn(Heard)> {
    on: F,
    rec: OfflineRecognizer,
    vad: VoiceActivityDetector,
    keep: Option<PathBuf>,
    always: bool,
    mode: Mode,
    held: Vec<f32>,
    voice: bool,
    // The last seconds of sound, so each sentence can start a little before the VAD noticed it (speech detection
    // reacts late, and the first word is usually her name), and so the last moment can be read while she talks.
    // `fed` counts every sample given to the VAD; segment starts count the same way.
    history: Vec<f32>,
    fed: i64,
    her: Option<Her>,
    /// Her last line, to tell her own voice in the mic.
    last_line: String,
    /// Sound that started before this sample may be her own voice: while she talks, and a moment after.
    hers_until: i64,
    /// Until when a yes or a no is an answer to her, without her name.
    answer_until: Option<Instant>,
    /// Her name was heard over her in the stretch of voice going on now: her line's name-like words (which don't
    /// count), the sample where the moment it was heard in began, and the sample where she went quiet. The sentence
    /// it ends up in carries the request.
    barged: Option<(Vec<String>, i64, i64)>,
    last_peek: Instant,
    pending: Option<Pending>,
    /// Listening in conversation (after her answer, no name needed), not after her name.
    chatting: bool,
    /// When her voice last stopped (what the speakers played before that was her).
    her_ended: Option<Instant>,
}

fn run<F: Fn(Heard)>(rx: Receiver<Cmd>, mut s: State<F>, source: Source) {
    let mut mic: Option<Opened> = None;
    let (mut level_sum, mut level_n, mut last_level) = (0.0f32, 0usize, Instant::now());
    loop {
        // The mic is open only while something could be listened to.
        let want_mic = s.always || s.mode != Mode::Waiting || s.pending.is_some();
        if want_mic && mic.is_none() {
            match open(&source) {
                Ok(m) => {
                    (s.on)(Heard::Mic { open: true, name: m.1.clone() });
                    mic = Some(m);
                }
                Err(e) => {
                    (s.on)(Heard::Error { message: format!("{e:#}") });
                    s.always = false;
                    s.mode = Mode::Waiting;
                    s.pending = None;
                }
            }
        } else if !want_mic && mic.is_some() {
            mic = None;
            s.forget();
            (s.on)(Heard::Mic { open: false, name: String::new() });
        }

        // Commands first: without blocking while the mic runs, waiting for one while it's closed. A closed channel
        // means the app is going away.
        let cmd = if mic.is_some() {
            match rx.try_recv() {
                Ok(c) => Some(c),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };
        if let Some(cmd) = cmd {
            s.command(cmd);
            continue;
        }
        let Some((_, _, chunks)) = &mic else { continue };
        let chunk = match chunks.recv_timeout(Duration::from_millis(200)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => {
                s.tick();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => {
                (s.on)(Heard::Error { message: "the microphone went away".into() });
                mic = None;
                continue;
            }
        };
        level_sum += chunk.iter().map(|x| x * x).sum::<f32>();
        level_n += chunk.len();
        if last_level.elapsed() >= Duration::from_millis(250) {
            (s.on)(Heard::Level { rms: (level_sum / level_n.max(1) as f32).sqrt() });
            (level_sum, level_n, last_level) = (0.0, 0, Instant::now());
        }
        s.hear(chunk);
        s.tick();
    }
}

impl<F: Fn(Heard)> State<F> {
    fn new(rec: OfflineRecognizer, vad: VoiceActivityDetector, keep: Option<PathBuf>, on: F) -> Self {
        Self {
            on,
            rec,
            vad,
            keep,
            always: false,
            mode: Mode::Waiting,
            held: Vec::new(),
            voice: false,
            history: Vec::with_capacity(HISTORY),
            fed: 0,
            her: None,
            last_line: String::new(),
            hers_until: 0,
            answer_until: None,
            barged: None,
            last_peek: Instant::now(),
            pending: None,
            chatting: false,
            her_ended: None,
        }
    }

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Always(v) => self.always = v,
            Cmd::Answering(d) => self.answer_until = (!d.is_zero()).then(|| Instant::now() + d),
            Cmd::Attend(d) => {
                self.pending = None;
                self.chatting = false;
                self.mode = Mode::Attending(Instant::now() + d);
            }
            Cmd::Converse(d) => {
                self.pending = None;
                self.chatting = true;
                self.mode = Mode::Attending(Instant::now() + d);
            }
            Cmd::Hold(true) => {
                self.held.clear();
                self.pending = None;
                self.mode = Mode::Holding;
            }
            Cmd::Hold(false) => {
                if self.mode == Mode::Holding {
                    self.mode = Mode::Waiting;
                    let t = Instant::now();
                    let text = transcribe(&self.rec, &self.held);
                    self.held.clear();
                    if text.is_empty() {
                        (self.on)(Heard::Timeout);
                    } else {
                        (self.on)(Heard::Request { text, after_name: false, ms: t.elapsed().as_millis() });
                    }
                }
            }
            Cmd::Speaking(Some(line)) => {
                self.her = Some(Her { names: wake::names_in(&line), since: Instant::now() });
                self.last_line = line;
                self.hers_until = i64::MAX;
                self.barged = None;
            }
            Cmd::Speaking(None) => {
                self.her_ended = Some(Instant::now());
                // (Already let go of when her name cut her off.)
                if self.her.take().is_some() {
                    self.hers_until = self.fed + ECHO_TAIL;
                }
            }
            Cmd::StopListening => {
                if matches!(self.mode, Mode::Attending(_)) {
                    self.mode = Mode::Waiting;
                }
                self.chatting = false;
                self.pending = None;
                self.barged = None;
            }
        }
    }

    /// The mic closed: what was half-heard goes.
    fn forget(&mut self) {
        self.vad.flush();
        self.vad.clear();
        self.voice = false;
        self.pending = None;
        self.barged = None;
    }

    fn hear(&mut self, chunk: Vec<f32>) {
        if self.mode == Mode::Holding {
            self.held.extend_from_slice(&chunk);
            return;
        }
        self.vad.accept_waveform(&chunk);
        self.history.extend_from_slice(&chunk);
        self.fed += chunk.len() as i64;
        if self.history.len() > HISTORY {
            self.history.drain(..self.history.len() - HISTORY);
        }
        if self.vad.detected() != self.voice {
            self.voice = !self.voice;
            (self.on)(Heard::Voice { on: self.voice });
        }
        if self.her.is_some() && self.voice && self.barged.is_none() && self.last_peek.elapsed() >= PEEK_EVERY {
            self.peek();
        }
        while let Some(seg) = self.vad.front() {
            let (start, n) = (seg.start(), seg.n() as i64);
            let samples = with_preroll(&self.history, self.fed, start, n).unwrap_or_else(|| seg.samples().to_vec());
            drop(seg);
            self.vad.pop();
            self.sentence(start, start + n, samples);
        }
    }

    /// While she talks: reads the last moment of sound for her name, so it cuts her off at once.
    fn peek(&mut self) {
        self.last_peek = Instant::now();
        let n = PEEK.min(self.history.len());
        if n < mic::RATE as usize / 2 {
            return;
        }
        let t = Instant::now();
        let text = transcribe(&self.rec, &self.history[self.history.len() - n..]);
        let ms = t.elapsed().as_millis();
        tracing::debug!("peek ({ms} ms): {text}");
        let names = self.her.as_ref().map(|h| h.names.clone()).unwrap_or_default();
        if wake::after_name(&text, &names).is_some() {
            self.barge_in(names, self.fed - n as i64);
            (self.on)(Heard::BargeIn { text, ms });
        }
    }

    /// Her name over her voice (heard in the sound from sample `from`): she's being stopped, and listens.
    fn barge_in(&mut self, names: Vec<String>, from: i64) {
        self.her = None;
        self.her_ended = Some(Instant::now());
        self.chatting = false;
        self.hers_until = self.fed;
        self.barged = Some((names, from, self.fed));
        self.pending = None;
        self.mode = Mode::Attending(Instant::now() + LISTEN_FOR);
    }

    fn sentence(&mut self, start: i64, end: i64, mut samples: Vec<f32>) {
        let mut from = start;
        // Started while she talked and went on well after she stopped: the part after her is someone else's (an
        // answer to her question, begun before her echo had died down).
        if self.barged.is_none() && self.her.is_none() && start < self.hers_until && end - self.hers_until > mic::RATE as i64 / 2
            && let Some(after) = slice(&self.history, self.fed, self.hers_until, end)
        {
            samples = after;
            from = self.hers_until;
        }
        // Her name cut her off in this sentence: it started in her voice, and two voices at once read as hers. Your
        // part is read from the moment her name was heard in.
        if let Some((_, named, _)) = self.barged
            && start < named
            && let Some(yours) = slice(&self.history, self.fed, named, end)
        {
            samples = yours;
            from = named;
        }
        let t = Instant::now();
        let text = transcribe(&self.rec, &samples);
        let ms = t.elapsed().as_millis();
        if text.is_empty() {
            return;
        }
        let (verdict, then) = self.decide(from, end, &samples, &text, ms);
        let audio = self.keep.as_deref().and_then(|d| save(d, &samples));
        (self.on)(Heard::Utterance { text, ms, verdict, audio });
        for h in then {
            (self.on)(h);
        }
    }

    /// What a sentence is: a request, her name, her own voice, nothing for her. Changes the mode to match, and
    /// returns what to tell the app after the sentence itself.
    fn decide(&mut self, start: i64, end: i64, samples: &[f32], text: &str, ms: u128) -> (&'static str, Vec<Heard>) {
        // She's waiting for a yes or a no: it's taken however it's said, without her name, even over her voice asking
        // ("Okay." was a stray sound, "Yes." wasn't for her). Only a yes with your speakers playing needs her name
        // ("Capri, yes"): a video saying "okay" isn't you agreeing to something risky.
        if self.answer_until.is_some_and(|t| Instant::now() < t)
            && let Some(yes) = capri_core::answer::yes_no(text)
        {
            if yes && start >= self.hers_until && self.speakers_played(start, end) && wake::find(text).is_none() {
                return ("a yes with your speakers playing (say her name with it); still waiting", vec![]);
            }
            self.barged = None;
            self.pending = None;
            self.chatting = false;
            self.mode = Mode::Waiting;
            return ("your answer to her question", vec![Heard::Request { text: text.to_string(), after_name: false, ms }]);
        }
        // Her name was heard over her voice in this stretch: the request is what follows it.
        if let Some((names, _, quiet)) = self.barged.take() {
            if let Some(rest) = wake::after_name(text, &names).filter(|r| wake::meaningful(r) && wake::english(r)) {
                return self.request(rest, samples, true, ms);
            }
            // Your words after her name drowned in hers ("Capri, on mute" in one breath, over her): from the moment
            // she went quiet, they're yours alone.
            if end - quiet > mic::RATE as i64 * 3 / 10
                && let Some(after) = slice(&self.history, self.fed, quiet, end)
            {
                let heard = transcribe(&self.rec, &after);
                tracing::debug!("after she went quiet: {heard}");
                let heard = wake::find(&heard).filter(|r| !r.is_empty()).unwrap_or(heard);
                if self.sounds_like_media(quiet, end) {
                    return ("her name, over her voice; then your speakers, it sounds like (not taken)", vec![]);
                }
                if wake::meaningful(&heard) && wake::english(&heard) {
                    return match self.request(heard, &after, true, ms) {
                        ("request", then) => ("her name, over her voice; the request from after she went quiet", then),
                        other => other,
                    };
                }
            }
            return ("her name, over her voice", vec![]);
        }
        // Started while she was talking: only her name counts.
        if start < self.hers_until {
            let names = self.her.as_ref().map_or_else(|| wake::names_in(&self.last_line), |h| h.names.clone());
            if let Some(rest) = wake::after_name(text, &names) {
                // Her name in a sentence the quick looks missed (a short one): the same as cutting her off.
                self.barge_in(names, start);
                self.barged = None;
                let mut then = vec![Heard::BargeIn { text: text.into(), ms }];
                if wake::meaningful(&rest) && wake::english(&rest) {
                    let (verdict, more) = self.request(rest, samples, true, ms);
                    then.extend(more);
                    return (verdict, then);
                }
                return ("her name, over her voice", then);
            }
            return (if wake::echo_share(text, &self.last_line) >= 0.5 { "her own voice" } else { "said while she talked, no name" }, vec![]);
        }
        match self.mode {
            Mode::Attending(_) => {
                // Her name again while she's listening ("Capri… Capri?"): she listens on, from now. (It was a "stray
                // sound", and her window closed on you.)
                if self.pending.is_none() && wake::find(text).is_some_and(|r| r.is_empty()) {
                    self.mode = Mode::Attending(Instant::now() + LISTEN_FOR);
                    self.chatting = false;
                    return ("her name", vec![Heard::Wake]);
                }
                // In conversation: what your speakers said isn't you (a video, a call). Her name still works.
                if self.chatting && wake::find(text).is_none() && self.speakers_played(start, end) {
                    return ("your speakers, in conversation (her name needed); still listening", vec![]);
                }
                // A name in front of the request is fine: "Capri, open Steam" while she's already listening.
                let mut text = wake::find(text).filter(|r| !r.is_empty()).unwrap_or_else(|| text.to_string());
                let mut samples = samples.to_vec();
                let mut after_name = false;
                if let Some(p) = self.pending.take() {
                    // The rest of a sentence that stopped mid-thought: both halves read together read best.
                    samples = [p.samples.as_slice(), &[0.0; mic::RATE as usize / 10], samples.as_slice()].concat();
                    let whole = transcribe(&self.rec, &samples);
                    let whole = wake::find(&whole).filter(|r| !r.is_empty()).unwrap_or(whole);
                    text = if wake::meaningful(&whole) { whole } else { format!("{} {text}", p.text) };
                    after_name = p.after_name;
                }
                // "Uh.", "T.", "Okay, my": not a request; keep listening until the window ends.
                if !wake::meaningful(&text) {
                    return ("a stray sound; still listening", vec![]);
                }
                if !wake::english(&text) {
                    return ("not English; still listening", vec![]);
                }
                if self.sounds_like_media(start, end) {
                    return ("long, with your speakers playing: a video or a call, it sounds like; still listening", vec![]);
                }
                self.request(text, &samples, after_name, ms)
            }
            // Her name at the start, or at the end set off by a comma ("Press enter, Capri.").
            Mode::Waiting => match wake::find(text).or_else(|| wake::trailing(text)) {
                Some(rest) if wake::meaningful(&rest) && wake::english(&rest) => self.request(rest, samples, true, ms),
                Some(_) => {
                    self.mode = Mode::Attending(Instant::now() + LISTEN_FOR);
                    self.chatting = false;
                    ("her name", vec![Heard::Wake])
                }
                None => ("not for her", vec![]),
            },
            Mode::Holding => ("push-to-talk", vec![]),
        }
    }

    /// A request, unless it stopped mid-thought: then it waits a moment for the rest.
    fn request(&mut self, text: String, samples: &[f32], after_name: bool, ms: u128) -> (&'static str, Vec<Heard>) {
        if wake::unfinished(&text) && samples.len() < LONGEST {
            let until = Instant::now() + REST_OF_IT;
            self.pending = Some(Pending { samples: samples.to_vec(), text, after_name, until });
            self.mode = Mode::Attending(until);
            return ("unfinished; waiting for the rest", vec![]);
        }
        self.mode = Mode::Waiting;
        self.chatting = false;
        ("request", vec![Heard::Request { text, after_name, ms }])
    }

    /// Whether your speakers were playing through most of the sentence from sample `start` to `end` (after her own
    /// voice stopped: that was her).
    fn speakers_played(&self, start: i64, end: i64) -> bool {
        self.speakers_share(start, end) > 0.3
    }

    /// A long stretch with your speakers playing through nearly all of it: a video or a call talking, not you (a
    /// 12-second YouTube super-chat read-out was taken as a request after her name). Short ones still count: "Capri,
    /// pause" with music on.
    fn sounds_like_media(&self, start: i64, end: i64) -> bool {
        end - start >= mic::RATE as i64 * 6 && self.speakers_share(start, end) > 0.6
    }

    fn speakers_share(&self, start: i64, end: i64) -> f32 {
        let now = Instant::now();
        let at = |s: i64| now.checked_sub(Duration::from_secs_f64((self.fed - s).max(0) as f64 / mic::RATE as f64)).unwrap_or(now);
        let mut from = at(start);
        if let Some(h) = self.her_ended {
            from = from.max(h + Duration::from_millis(400));
        }
        let to = at(end);
        if to > from { crate::beat::loud_share(from, to) } else { 0.0 }
    }

    /// Time passing: a pending request goes as it is, a listening window closes.
    fn tick(&mut self) {
        let now = Instant::now();
        // A line never reported finished (it shouldn't happen): her voice isn't in the mic forever.
        if self.her.as_ref().is_some_and(|h| h.since.elapsed() > Duration::from_secs(120)) {
            self.her = None;
            self.hers_until = self.fed;
        }
        if let Some(p) = &self.pending {
            if now > p.until && (!self.voice || now > p.until + MORE) {
                let p = self.pending.take().expect("checked above");
                self.mode = Mode::Waiting;
                (self.on)(Heard::Request { text: p.text, after_name: p.after_name, ms: 0 });
            }
            return;
        }
        // The window closes when it's over and nobody's mid-sentence; a sentence that started in time gets `MORE`.
        if let Mode::Attending(until) = self.mode
            && now > until
            && (!self.voice || now > until + MORE)
        {
            self.mode = Mode::Waiting;
            self.barged = None;
            self.chatting = false;
            (self.on)(Heard::Timeout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PREROLL, slice, with_preroll};

    #[test]
    fn preroll_reaches_back() {
        let history: Vec<f32> = (0..20_000).map(|i| i as f32).collect();
        // history holds samples 10_000..30_000
        let got = with_preroll(&history, 30_000, 20_000, 1000).unwrap();
        assert_eq!(got.len() as i64, 1000 + PREROLL);
        assert_eq!(got[0] as i64, 20_000 - PREROLL - 10_000);
        assert!(with_preroll(&history, 30_000, 5_000, 1000).is_none(), "older than the history");
        // Right after the start of the history, the preroll is what there is.
        assert_eq!(with_preroll(&history, 30_000, 10_100, 100).unwrap().len(), 200);
    }

    #[test]
    fn slices_by_sample_count() {
        let history: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        // history holds samples 5_000..6_000
        assert_eq!(slice(&history, 6_000, 5_500, 5_510).unwrap()[0], 500.0);
        assert!(slice(&history, 6_000, 4_000, 5_100).is_none());
        assert!(slice(&history, 6_000, 5_900, 6_100).is_none());
    }
}
