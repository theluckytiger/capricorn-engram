//! Her hands: every tool she (or an MCP client) can use. OS-specific code lives only in `platform`.

pub mod controls;
pub mod eyes;
pub mod files;
pub mod games;
pub mod matching;
pub mod remind;
pub mod music;
pub mod platform;
pub mod read;
pub mod situation;
pub mod system;
pub mod library;
pub mod tool;
pub mod weather;
pub mod web;

#[cfg(windows)]
pub mod discord;
#[cfg(windows)]
pub mod pc;
#[cfg(windows)]
pub mod look;
#[cfg(windows)]
pub mod point;
#[cfg(windows)]
pub mod power;
#[cfg(windows)]
pub mod shell;
#[cfg(windows)]
pub mod watch;
#[cfg(windows)]
pub mod shot;

pub use tool::{Aside, Ctx, Hands, Outcome, Risk, Status, Tool, ToolSpec};

/// Every tool she has on this OS.
pub fn all() -> Hands {
    let mut h = Hands::default();
    h.add(weather::Weather::new()).add(weather::SetHome).add(web::Web).add(remind::Remind).add(files::Files).add(read::ReadFile);
    #[cfg(windows)]
    {
        h.add(pc::Windows).add(pc::Open).add(pc::Search).add(pc::Media).add(pc::Volume).add(pc::Keys).add(pc::TypeText).add(pc::Tab);
        h.add(controls::ReadWindow).add(controls::Click).add(controls::TypeIn).add(discord::Discord).add(eyes::See).add(music::PlayMusic).add(system::System).add(shot::Screenshot).add(shell::Shell).add(look::Look).add(look::ClickAt).add(look::DragAt).add(look::Scroll).add(watch::Watch).add(watch::WatchAlong).add(point::PointOut).add(power::Power);
    }
    h
}
