//! Her ear for music: what your speakers play (WASAPI loopback, in memory only, never kept), and whether it has a
//! steady beat. The low end's rises in energy make an onset curve at 100 Hz; every second, the last 6 s of it are
//! autocorrelated over 70–170 BPM, and only a clear, repeated peak counts, so a video's talking has no beat and music
//! does. She dances to it.

use std::collections::VecDeque;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

/// A beat she can dance to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Beat {
    pub bpm: f32,
    /// How sure (0..1).
    pub confidence: f32,
    /// How long ago the last beat was, ms (to start on the beat).
    pub since_ms: u32,
}

/// What your speakers played lately: when, and how loud (RMS), a hundred times a second for the last 30 s. In memory
/// only. In conversation mode a sentence heard while they played isn't taken without her name (a video, a call).
static LOUD: Mutex<VecDeque<(Instant, f32)>> = Mutex::new(VecDeque::new());
/// Louder than this, something's playing (about -40 dBFS).
const PLAYING: f32 = 0.01;

/// How much of `from..to` your speakers were playing something (0..1); 0 when her ear on them is off.
pub fn loud_share(from: Instant, to: Instant) -> f32 {
    let l = LOUD.lock().unwrap();
    let (n, loud) = l.iter().filter(|(t, _)| *t >= from && *t <= to).fold((0u32, 0u32), |(n, k), (_, v)| (n + 1, k + u32::from(*v > PLAYING)));
    if n == 0 { 0.0 } else { loud as f32 / n as f32 }
}

/// Onset curve samples a second.
const RATE: usize = 100;
/// How much of it is looked at.
const WINDOW: usize = RATE * 6;

/// The same beat heard again: within 6%, or half or double it (an ear's octave slip, 143 heard as 71): she keeps
/// dancing as she was instead of starting over.
fn same_tempo(a: f32, b: f32) -> bool {
    let r = a.max(b) / a.min(b).max(1.0);
    r < 1.06 || (1.88..2.12).contains(&r)
}

/// Onset curve → the beat in it, if there's a clear one.
pub fn find(onsets: &[f32]) -> Option<Beat> {
    find_over(onsets, 0.28)
}

/// The same, taking a beat as sure as `min` (lower once she's dancing: a song's quieter bars keep her going).
fn find_over(onsets: &[f32], min: f32) -> Option<Beat> {
    if onsets.len() < RATE * 3 {
        return None;
    }
    let mean = onsets.iter().sum::<f32>() / onsets.len() as f32;
    let x: Vec<f32> = onsets.iter().map(|v| v - mean).collect();
    let zero: f32 = x.iter().map(|v| v * v).sum();
    if zero < 1e-9 {
        return None;
    }
    let ac = |lag: usize| x.iter().zip(&x[lag..]).map(|(a, b)| a * b).sum::<f32>() / zero;
    // 70–170 BPM; a lag and its double both fitting is how a real beat looks.
    let (lo, hi) = (RATE * 60 / 170, RATE * 60 / 70);
    let (lag, score) = (lo..=hi).map(|l| (l, ac(l) + 0.5 * ac((l * 2).min(x.len() - 1)))).max_by(|a, b| a.1.total_cmp(&b.1))?;
    let confidence = (score / 1.5).clamp(0.0, 1.0);
    if confidence < min {
        return None;
    }
    // Where the beats fall: the offset whose every-`lag` comb collects the most onset, counted back from now.
    let n = x.len();
    let since = (0..lag).max_by(|&a, &b| {
        let comb = |o: usize| (0..n / lag).filter_map(|k| n.checked_sub(1 + o + k * lag)).map(|i| onsets[i]).sum::<f32>();
        comb(a).total_cmp(&comb(b))
    })?;
    Some(Beat { bpm: 60.0 * RATE as f32 / lag as f32, confidence, since_ms: (since * 1000 / RATE) as u32 })
}

/// Keeps her ear on the speakers; `on` gets a beat when one starts or changes, and `None` when it stops.
/// The stream lives on its own thread; dropping the handle stops it.
pub struct Listening {
    stop: mpsc::Sender<()>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

pub fn start(on: impl Fn(Option<Beat>) + Send + 'static) -> Result<Listening> {
    let (stop, stopped) = mpsc::channel::<()>();
    let (tx, rx) = mpsc::channel::<f32>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
    std::thread::Builder::new().name("beat-stream".into()).spawn(move || {
        let stream = match open(tx) {
            Ok(s) => {
                let _ = ready_tx.send(Ok(()));
                s
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        let _ = stopped.recv();
        drop(stream);
    })?;
    ready_rx.recv().map_err(|_| anyhow!("the beat thread died"))??;
    std::thread::Builder::new().name("beat".into()).spawn(move || {
        let mut onsets: Vec<f32> = Vec::with_capacity(WINDOW + RATE);
        let mut last = Instant::now();
        let (mut now_beat, mut yes, mut no): (Option<Beat>, u32, u32) = (None, 0, 0);
        while let Ok(v) = rx.recv() {
            onsets.push(v);
            if onsets.len() > WINDOW {
                onsets.drain(..onsets.len() - WINDOW);
            }
            if last.elapsed() < Duration::from_secs(1) {
                continue;
            }
            last = Instant::now();
            // Two clear seconds to start dancing (one when it's very clear); dancing, a fainter beat keeps her going
            // and only eight seconds without one stops her (a song's break), or two of silence (it stopped).
            match find_over(&onsets, if now_beat.is_some() { 0.16 } else { 0.28 }) {
                Some(b) => {
                    (yes, no) = (yes + 1, 0);
                    let changed = now_beat.is_none_or(|n| !same_tempo(n.bpm, b.bpm));
                    if (yes >= 2 || b.confidence >= 0.5) && changed {
                        if now_beat.is_none() {
                            tracing::info!("beat: {:.0} BPM ({:.2})", b.bpm, b.confidence);
                        }
                        now_beat = Some(b);
                        on(Some(b));
                    }
                }
                None => {
                    (yes, no) = (0, no + 1);
                    let silent = onsets.len() >= RATE * 2 && onsets[onsets.len() - RATE * 2..].iter().all(|v| *v < 1e-5);
                    if (no >= 8 || (silent && no >= 2)) && now_beat.take().is_some() {
                        tracing::info!("beat: gone");
                        on(None);
                    }
                    // Something playing without a beat she takes: what she made of it, now and then (for tuning).
                    if no % 10 == 0
                        && let Some(b) = find_over(&onsets, 0.0)
                    {
                        tracing::info!("beat: none clear (best {:.0} BPM at {:.2})", b.bpm, b.confidence);
                    }
                }
            }
        }
    })?;
    Ok(Listening { stop })
}

/// The speakers as an input (loopback): their sound becomes the onset curve, 100 values a second.
fn open(tx: mpsc::Sender<f32>) -> Result<cpal::Stream> {
    let device = cpal::default_host().default_output_device().ok_or_else(|| anyhow!("no speakers"))?;
    let config = device.default_output_config()?;
    let channels = config.channels() as usize;
    let hop = (config.sample_rate() as usize / RATE).max(1);
    // Two bands: a low-pass for the kick and bass, and what's above a few kHz for the hats (a garage or trap beat's
    // steady pulse is often there, its kicks swung). Each hop's energy in each; an onset is how much they rose since
    // the last hop, the hats' scaled to the bass's usual level so neither drowns the other.
    let (mut lp, mut energy, mut raw, mut count, mut prev) = (0.0f32, 0.0f32, 0.0f32, 0usize, 0.0f32);
    let (mut mid, mut hi_energy, mut prev_hi, mut avg_lo, mut avg_hi) = (0.0f32, 0.0f32, 0.0f32, 1e-4f32, 1e-4f32);
    let mut push = move |mono: f32| {
        lp += 0.05 * (mono - lp);
        mid += 0.3 * (mono - mid);
        let hi = mono - mid;
        energy += lp * lp;
        hi_energy += hi * hi;
        raw += mono * mono;
        count += 1;
        if count >= hop {
            let e = (energy / count as f32).sqrt();
            let h = (hi_energy / count as f32).sqrt();
            avg_lo += 0.01 * (e - avg_lo);
            avg_hi += 0.01 * (h - avg_hi);
            let onset = (e - prev).max(0.0) + 0.7 * (h - prev_hi).max(0.0) * (avg_lo / avg_hi.max(1e-6)).min(20.0);
            let _ = tx.send(onset);
            prev = e;
            prev_hi = h;
            // How loud, kept for a while (never waiting on the lock here: this is the audio thread).
            if let Ok(mut l) = LOUD.try_lock() {
                let now = Instant::now();
                l.push_back((now, (raw / count as f32).sqrt()));
                while l.front().is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(30)) {
                    l.pop_front();
                }
            }
            (energy, hi_energy, raw, count) = (0.0, 0.0, 0.0, 0);
        }
    };
    let err = |e| tracing::debug!("beat: {e}");
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(config.into(), move |d: &[f32], _: &_| d.chunks(channels).for_each(|f| push(f.iter().sum::<f32>() / channels as f32)), err, None)?,
        cpal::SampleFormat::I16 => device.build_input_stream(config.into(), move |d: &[i16], _: &_| d.chunks(channels).for_each(|f| push(f.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32)), err, None)?,
        other => return Err(anyhow!("speaker format {other:?} isn't supported")),
    };
    stream.play()?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kick_at_120_is_found() {
        // 120 BPM: a kick every 50 onset samples, with noise.
        let mut seed = 7u32;
        let mut rand = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as f32 / 65_536.0
        };
        let onsets: Vec<f32> = (0..WINDOW).map(|i| if i % 50 == 0 { 1.0 } else { 0.0 } + rand() * 0.1).collect();
        let b = find(&onsets).expect("a beat");
        assert!((b.bpm - 120.0).abs() < 3.0, "{b:?}");
    }

    #[test]
    fn talk_has_no_beat() {
        // Onsets at random times: speech.
        let mut seed = 11u32;
        let mut rand = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as f32 / 65_536.0
        };
        let onsets: Vec<f32> = (0..WINDOW).map(|_| if rand() < 0.06 { rand() } else { 0.0 }).collect();
        assert!(find(&onsets).is_none());
    }

    #[test]
    fn half_and_double_are_the_same_beat() {
        assert!(super::same_tempo(143.0, 71.0) && super::same_tempo(143.0, 146.0) && !super::same_tempo(143.0, 120.0));
    }

    #[test]
    fn three_seconds_of_a_kick_is_enough() {
        let onsets: Vec<f32> = (0..RATE * 3).map(|i| if i % 50 == 0 { 1.0 } else { 0.0 }).collect();
        let b = find(&onsets).expect("a beat");
        assert!((b.bpm - 120.0).abs() < 3.0, "{b:?}");
    }
}
