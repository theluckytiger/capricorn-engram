//! Her brain: providers (local and cloud), the router, the fast path and the agent loop.

pub mod agent;
pub mod eyes;
pub mod fastpath;
pub mod lines;
pub mod llm;
pub mod persona;
pub mod providers;
pub mod router;

pub use agent::{DenyRisky, Events, Gate, NoEvents, StepEvent};
pub use router::{Answer, Brain};
