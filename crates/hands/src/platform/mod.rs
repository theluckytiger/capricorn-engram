//! The only OS-specific code. Everything above this module talks to these types and functions.

use serde::Serialize;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

/// A top-level window as she sees it.
#[derive(Debug, Clone, Serialize)]
pub struct WinInfo {
    /// The OS handle, as a number (stable while the window lives).
    pub id: isize,
    pub title: String,
    /// The program's friendly name ("Discord", "Brave Browser"), from its file description, else its exe name.
    pub app: String,
    /// The exe name without `.exe`, lowercase ("discord", "brave", "steamwebhelper").
    pub exe: String,
    pub pid: u32,
    /// left, top, right, bottom in screen pixels.
    pub rect: [i32; 4],
    pub minimized: bool,
    pub maximized: bool,
    pub foreground: bool,
}

/// What's playing, from the OS's media sessions.
#[derive(Debug, Clone, Serialize)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// The app playing it ("Brave", "Spotify"…).
    pub app: String,
    pub playing: bool,
}

/// A control inside a window (UI Automation): what it's called, what kind it is, where it is.
#[derive(Debug, Clone, Serialize)]
pub struct Control {
    /// Its place in the window's list, for "the second one".
    pub index: usize,
    pub name: String,
    /// "button", "field", "link", "tab", "checkbox", "list item"…
    pub kind: String,
    /// left, top, right, bottom in screen pixels.
    pub rect: [i32; 4],
    pub enabled: bool,
    /// On/off, for switches and toggle buttons (Discord's mic is a switch called "Mute" either way).
    pub toggled: Option<bool>,
    pub focused: bool,
    /// The app's own id for it, when it has one.
    pub id: String,
}

impl Control {
    pub fn center(&self) -> (i32, i32) {
        ((self.rect[0] + self.rect[2]) / 2, (self.rect[1] + self.rect[3]) / 2)
    }
}

/// An app she can open: its name as the Start menu shows it, and how to launch it.
#[derive(Debug, Clone, Serialize)]
pub struct AppEntry {
    pub name: String,
    pub id: String,
}
