//! Her ears: the mic, voice activity, her names, speech to text. All on this PC, on the CPU.

pub mod beat;
pub mod engine;
pub mod mic;
pub mod wake;

pub use engine::{Config, Ears, Heard, models_ok, start};
