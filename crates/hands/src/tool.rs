//! What a tool is: a name, a JSON schema for its arguments, how risky a call is, and what happened when it ran.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool as the model sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments object.
    pub parameters: Value,
}

/// How much a call could cost you if it went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Only looks: list windows, what's playing, the weather.
    Read,
    /// Changes something easily undone: open, focus, click, type, media, volume.
    Normal,
    /// Sends, deletes, buys, installs, runs a program file or a script, shuts down: asks first unless Blackwall.
    Risky,
    /// Money, wiping data, Windows security, code from the internet: always asks.
    NeverSilent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// It happened, and the tool checked.
    Ok,
    /// It didn't happen; `summary` says why.
    Failed,
    /// It was tried, but the tool couldn't confirm the result.
    Unsure,
}

/// What a call did. `summary` is written for the model (and ends up behind her words), `data` for code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub status: Status,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

impl Outcome {
    pub fn ok(summary: impl Into<String>) -> Self {
        Self { status: Status::Ok, summary: summary.into(), data: Value::Null }
    }
    pub fn failed(summary: impl Into<String>) -> Self {
        Self { status: Status::Failed, summary: summary.into(), data: Value::Null }
    }
    pub fn unsure(summary: impl Into<String>) -> Self {
        Self { status: Status::Unsure, summary: summary.into(), data: Value::Null }
    }
    pub fn with(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}

impl From<anyhow::Error> for Outcome {
    fn from(e: anyhow::Error) -> Self {
        Outcome::failed(format!("{e:#}"))
    }
}

/// Where a tool sends something she noticed on her own, to be said her way (watching a video with you).
#[derive(Clone)]
pub struct Aside(pub std::sync::Arc<dyn Fn(String) + Send + Sync>);

impl std::fmt::Debug for Aside {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Aside")
    }
}

/// What a tool knows about the moment it runs in.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    /// "Home" for the weather and "at home": a place name.
    pub home: Option<String>,
    /// Her own windows (by process id), so she never acts on herself.
    pub own_pid: u32,
    /// In dev: act only inside these windows (handles); everything else is pointed at, not touched.
    pub dry_run: bool,
    /// Her vision model, when she has one she may show the screen to.
    pub eyes: Option<crate::eyes::Eyes>,
    /// A stronger vision model in the cloud she may NOT show the screen to unless you say yes, each time: when her
    /// own eyes can't make sense of a window she asks, and on a yes that one look goes there.
    pub cloud_eyes: Option<(crate::eyes::Eyes, String)>,
    /// This call is the look you said yes to: it goes to `cloud_eyes`.
    pub use_cloud_eyes: bool,
    /// Pictures of the screen may go to a cloud brain without asking each job (her Privacy setting).
    pub screens_to_cloud: bool,
    /// Something she noticed on her own, for her to say (watching along).
    pub aside: Option<Aside>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    /// The risk of this particular call (a shell command that only reads is `Read`, one that deletes is `Risky`).
    fn risk(&self, _args: &Value) -> Risk {
        Risk::Normal
    }
    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome;
}

/// All her tools.
#[derive(Clone, Default)]
pub struct Hands {
    tools: Vec<Arc<dyn Tool>>,
}

impl Hands {
    pub fn add(&mut self, tool: impl Tool + 'static) -> &mut Self {
        self.tools.push(Arc::new(tool));
        self
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|t| t.spec()).collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.spec().name == name).cloned()
    }

    /// Runs a call. An unknown tool or a bad argument is a failed outcome the model can read and correct.
    pub async fn call(&self, name: &str, args: Value, ctx: &Ctx) -> Outcome {
        match self.get(name) {
            Some(t) => t.call(args, ctx).await,
            None => Outcome::failed(format!("there is no tool called `{name}`")),
        }
    }
}

/// Reads one argument, or says which one is missing in words the model understands.
pub fn arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, Outcome> {
    match args.get(key) {
        Some(v) if !v.is_null() => loose(v).map_err(|e| Outcome::failed(format!("argument `{key}`: {e}"))),
        _ => Err(Outcome::failed(format!("missing argument `{key}`"))),
    }
}

pub fn opt_arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Option<T> {
    args.get(key).filter(|v| !v.is_null()).and_then(|v| loose(v).ok())
}

/// A value as `T`, taking a number or true/false where text is wanted (a brain sent `"text": 80` for a slider), and
/// a number written as text where a number is wanted.
fn loose<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, serde_json::Error> {
    serde_json::from_value(v.clone()).or_else(|e| match v {
        Value::Number(_) | Value::Bool(_) => serde_json::from_value(Value::String(v.to_string())).map_err(|_| e),
        Value::String(s) => s.trim().parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).and_then(|n| serde_json::from_value(n).ok()).ok_or(e),
        _ => Err(e),
    })
}
