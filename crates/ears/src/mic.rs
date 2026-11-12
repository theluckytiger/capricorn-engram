//! The microphone: the system's default input, as 16 kHz mono f32, in small chunks over a channel.
//!
//! cpal's stream has to stay on the thread that made it, so the mic lives on its own thread; dropping the returned
//! handle stops it and closes the device.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub const RATE: i32 = 16_000;

pub struct Mic {
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
    pub name: String,
}

impl Drop for Mic {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Opens the default mic. Chunks (about 64 ms each) arrive on the returned receiver.
pub fn open() -> Result<(Mic, Receiver<Vec<f32>>)> {
    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<String>>();
    let thread = std::thread::Builder::new()
        .name("mic".into())
        .spawn(move || {
            let stream = match build(tx) {
                Ok((s, name)) => {
                    let _ = ready_tx.send(Ok(name));
                    s
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = stop_rx.recv();
            drop(stream);
        })?;
    let name = ready_rx.recv().context("mic thread died")??;
    Ok((Mic { stop: stop_tx, thread: Some(thread), name }, rx))
}

fn build(tx: Sender<Vec<f32>>) -> Result<(cpal::Stream, String)> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| anyhow!("no microphone"))?;
    let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "microphone".into());
    let config = device.default_input_config().context("reading the mic's format")?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate() as i32;
    let resampler = if rate == RATE { None } else { Some(sherpa_onnx::LinearResampler::create(rate, RATE).ok_or_else(|| anyhow!("no resampler for {rate} Hz"))?) };
    let mut pending: Vec<f32> = Vec::with_capacity(4096);
    let chunk = (RATE as usize) / 16; // 62.5 ms
    let mut push = move |mono: Vec<f32>| {
        let mono = match &resampler {
            Some(r) => r.resample(&mono, false),
            None => mono,
        };
        pending.extend_from_slice(&mono);
        while pending.len() >= chunk {
            let rest = pending.split_off(chunk);
            let _ = tx.send(std::mem::replace(&mut pending, rest));
        }
    };
    let err = |e| tracing::warn!("mic: {e}");
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(config.into(), move |data: &[f32], _: &_| push(downmix(data, channels, |s| s)), err, None)?,
        cpal::SampleFormat::I16 => device.build_input_stream(config.into(), move |data: &[i16], _: &_| push(downmix(data, channels, |s| s as f32 / 32768.0)), err, None)?,
        cpal::SampleFormat::I32 => device.build_input_stream(config.into(), move |data: &[i32], _: &_| push(downmix(data, channels, |s| s as f32 / 2147483648.0)), err, None)?,
        other => return Err(anyhow!("mic sample format {other:?} isn't supported")),
    };
    stream.play()?;
    tracing::info!("mic: {name}, {rate} Hz, {channels} ch");
    Ok((stream, name))
}

fn downmix<T: Copy>(data: &[T], channels: usize, to_f32: impl Fn(T) -> f32) -> Vec<f32> {
    if channels <= 1 {
        return data.iter().map(|&s| to_f32(s)).collect();
    }
    data.chunks(channels).map(|frame| frame.iter().map(|&s| to_f32(s)).sum::<f32>() / channels as f32).collect()
}
