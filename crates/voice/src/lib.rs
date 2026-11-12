//! Her voice: text to speech on this PC, a sentence at a time, and playing it.

pub mod engine;
pub mod normalize;
pub mod sentences;
pub mod speaker;

pub use engine::{Engine, VoiceSpec};
pub use speaker::{Said, Speaker, start};
