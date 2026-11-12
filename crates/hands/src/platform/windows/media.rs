//! What's playing and the play/pause/next controls, through Windows' media sessions (the same thing the volume flyout
//! shows). Unlike media keys, a session is a specific player, so "pause" pauses what's actually playing.

use anyhow::{Context, Result, bail};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Playback,
};

use crate::platform::NowPlaying;

fn manager() -> Result<Manager> {
    super::keep_com_alive();
    Ok(Manager::RequestAsync()?.join()?)
}

fn sessions(m: &Manager) -> Result<Vec<Session>> {
    Ok(m.GetSessions()?.into_iter().collect())
}

fn is_playing(s: &Session) -> bool {
    s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|p| p == Playback::Playing).unwrap_or(false)
}

fn describe(s: &Session) -> Result<NowPlaying> {
    let props = s.TryGetMediaPropertiesAsync()?.join()?;
    let app = s.SourceAppUserModelId().map(|a| app_name(&a.to_string())).unwrap_or_default();
    Ok(NowPlaying { title: props.Title()?.to_string(), artist: props.Artist()?.to_string(), app, playing: is_playing(s) })
}

/// "Brave.ZWS7…" or "C:\…\Spotify.exe" or "Spotify.exe" → "Brave" / "Spotify".
fn app_name(aumid: &str) -> String {
    let last = aumid.rsplit(['\\', '!']).next().unwrap_or(aumid);
    let base = last.split('.').next().unwrap_or(last);
    let base = base.split('_').next().unwrap_or(base);
    let mut c = base.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => aumid.to_string(),
    }
}

/// Every media session, the playing ones first.
pub fn now_playing() -> Result<Vec<NowPlaying>> {
    let m = manager()?;
    let mut all: Vec<NowPlaying> = sessions(&m)?.iter().filter_map(|s| describe(s).ok()).collect();
    all.sort_by_key(|n| !n.playing);
    Ok(all)
}

/// play | pause | toggle | next | previous. Returns what's playing afterwards (or what was acted on).
pub fn media(action: &str) -> Result<NowPlaying> {
    let m = manager()?;
    let all = sessions(&m)?;
    if all.is_empty() {
        bail!("nothing is playing or paused in any app");
    }
    // Pausing and skipping act on what's playing; playing acts on Windows' current session (the most recent).
    let playing = all.iter().find(|s| is_playing(s)).cloned();
    let current = m.GetCurrentSession().ok();
    let target = match action {
        "pause" | "next" | "previous" => playing.clone().or(current.clone()),
        _ => current.clone().or(playing.clone()),
    }
    .or_else(|| all.first().cloned())
    .context("no media session to control")?;
    let done = match action {
        "play" => target.TryPlayAsync()?.join()?,
        "pause" => target.TryPauseAsync()?.join()?,
        "toggle" => target.TryTogglePlayPauseAsync()?.join()?,
        "next" => target.TrySkipNextAsync()?.join()?,
        "previous" => target.TrySkipPreviousAsync()?.join()?,
        other => bail!("unknown media action \"{other}\""),
    };
    if !done {
        bail!("the player refused \"{action}\"");
    }
    // Give the player a moment, then report what it says now.
    std::thread::sleep(std::time::Duration::from_millis(if matches!(action, "next" | "previous") { 900 } else { 350 }));
    describe(&target)
}

#[cfg(test)]
mod tests {
    use super::app_name;

    #[test]
    fn names() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe"), "Brave");
        assert_eq!(app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Microsoft");
    }
}
