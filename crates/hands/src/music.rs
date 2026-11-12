//! Playing a song (or any video) by name: YouTube's own results page gives the video, and it opens straight on it in
//! YouTube Music (or YouTube), so it plays at once: no search page to click through. Then Windows' media sessions
//! say whether it really started.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

/// The regular videos on a YouTube results page, in order: (id, title). Shorts, ads and playlists aren't
/// `videoRenderer`s, so they're skipped.
pub fn videos(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("\"videoRenderer\":{\"videoId\":\"") {
        rest = &rest[i + 28..];
        let Some(id) = rest.get(..11).filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) else { continue };
        let title = rest
            .find("\"title\":{\"runs\":[{\"text\":\"")
            .and_then(|t| {
                let s = &rest[t + 26..];
                s.find("\"}").map(|e| &s[..e])
            })
            .map(|t| serde_json::from_str::<String>(&format!("\"{t}\"")).unwrap_or_else(|_| t.to_string()))
            .unwrap_or_default();
        if !out.iter().any(|(x, _): &(String, String)| x == id) {
            out.push((id.to_string(), title));
        }
    }
    out
}

pub struct PlayMusic;

#[async_trait]
impl Tool for PlayMusic {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "play_music".into(),
            description: "Play a song, artist, mix or any video by name, right away: it opens playing in YouTube Music \
                          (the default), on YouTube, or in the Spotify app (`where: spotify`, when they say Spotify). \
                          `query`: what to play (\"Blinding Lights\", \"some phonk\", \"lofi beats\"). `nth`: the nth \
                          result instead of the first (\"the third one\")."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "where": { "type": "string", "enum": ["youtube_music", "youtube", "spotify"] },
                    "nth": { "type": "integer", "minimum": 1 }
                },
                "required": ["query"]
            }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Normal
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let query: String = match arg(&args, "query") {
            Ok(q) => q,
            Err(o) => return o,
        };
        let site: String = opt_arg(&args, "where").unwrap_or_else(|| "youtube_music".into());
        let nth: usize = opt_arg(&args, "nth").unwrap_or(1).max(1);
        if site == "spotify" {
            return spotify(&query, ctx).await;
        }
        // The results page is well over a megabyte: on a slow connection 8 s cut it off halfway, and a cut page read
        // as "0 results". Twice, with time to arrive.
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(25)).user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)").build().expect("http client");
        let mut html = String::new();
        for _ in 0..2 {
            match http.get("https://www.youtube.com/results").query(&[("search_query", query.as_str())]).send().await {
                Ok(r) => match r.text().await {
                    Ok(t) => {
                        html = t;
                        break;
                    }
                    Err(e) => tracing::warn!("YouTube's results page didn't come whole: {e}"),
                },
                Err(e) => tracing::warn!("couldn't reach YouTube: {e}"),
            }
        }
        if html.is_empty() {
            return Outcome::failed("couldn't get YouTube's results page (the connection is slow or down)");
        }
        let found = videos(&html);
        let Some((id, title)) = found.get(nth - 1).cloned() else {
            return Outcome::failed(format!("YouTube has {} results for \"{query}\", not a number {nth}", found.len()));
        };
        let (url, app) = if site == "youtube" { (format!("https://www.youtube.com/watch?v={id}"), "YouTube") } else { (format!("https://music.youtube.com/watch?v={id}"), "YouTube Music") };
        let data = json!({ "results": found.iter().take(5).map(|(i, t)| json!([i, t])).collect::<Vec<_>>(), "url": url });
        if ctx.dry_run {
            return Outcome::ok(format!("(dry run) would play \"{title}\" in {app}")).with(data);
        }
        #[cfg(windows)]
        {
            if let Err(e) = crate::pc::shell_open(&url) {
                return Outcome::failed(format!("couldn't open {app}: {e}")).with(data);
            }
            // It's playing when Windows' media sessions show it (a browser tab reports its title).
            // Compared with its title and artist together: YouTube Music reports "Firestarter" by "The Prodigy" for
            // "The Prodigy - Firestarter (Official Video)", and the video's first three words never matched.
            let words: Vec<String> = crate::matching::norm(&title).split(' ').filter(|w| w.len() > 2 && !FILLER.contains(w)).take(4).map(String::from).collect();
            let heard = |n: &crate::platform::NowPlaying| {
                let hay = crate::matching::norm(&format!("{} {}", n.title, n.artist));
                words.is_empty() || words.iter().filter(|w| hay.contains(w.as_str())).count() * 2 >= words.len()
            };
            for _ in 0..20 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let sessions = tokio::task::spawn_blocking(crate::platform::now_playing).await.ok().and_then(Result::ok).unwrap_or_default();
                if let Some(now) = sessions.iter().find(|n| n.playing && heard(n)) {
                    return Outcome::ok(format!("playing \"{}\" in {app}", now.title)).with(data);
                }
            }
            Outcome::unsure(format!("opened \"{title}\" in {app}; it hasn't shown up as playing yet (the browser may need a click to start sound)")).with(data)
        }
        #[cfg(not(windows))]
        Outcome::failed("not on this OS yet").with(data)
    }
}

/// Spotify: its search opened in the app, the top result's Play pressed (read from the app's own controls, else
/// found by looking), and checked playing in Windows' media sessions.
/// Words in a video's title that aren't the song: "(Official Video)", "Lyrics", "HD"…
const FILLER: &[&str] = &["the", "official", "video", "audio", "lyrics", "lyric", "music", "hd", "4k", "remastered", "live", "visualizer", "feat", "ft", "and"];

async fn spotify(query: &str, ctx: &Ctx) -> Outcome {
    if ctx.dry_run {
        return Outcome::ok(format!("(dry run) would play \"{query}\" in Spotify"));
    }
    #[cfg(windows)]
    {
        // Don't shell-open spotify: when the app isn't there: Windows shows a Store dialog and looks broken.
        let installed = tokio::task::spawn_blocking(|| crate::platform::list_windows().into_iter().any(|w| w.exe == "spotify") || crate::platform::apps().ok().into_iter().flatten().any(|a| a.name.to_lowercase().contains("spotify"))).await.ok().unwrap_or(false);
        if !installed {
            return Outcome::failed("Spotify isn't installed; use where youtube_music (or youtube) instead");
        }
        let uri = format!("spotify:search:{}", query.split_whitespace().collect::<Vec<_>>().join("%20"));
        if crate::pc::shell_open(&uri).is_err() {
            return Outcome::failed("Spotify doesn't seem to be installed; use where youtube_music instead");
        }
        let found = tokio::task::spawn_blocking(|| crate::platform::wait_for_window(|w| w.exe == "spotify" && !w.title.is_empty(), Duration::from_secs(10))).await.ok().flatten();
        let Some(w) = found else { return Outcome::failed("Spotify didn't open (is it installed?). Use where youtube_music instead") };
        // The results take a moment to show.
        tokio::time::sleep(Duration::from_millis(1800)).await;
        let id = w.id;
        let play = tokio::task::spawn_blocking(move || {
            let list = crate::platform::controls(id, 600).ok()?;
            let c = list.into_iter().find(|c| c.kind == "button" && c.name.to_lowercase().starts_with("play ") && c.name.len() > 6)?;
            crate::platform::press_control(id, &c).ok().map(|_| c.name)
        })
        .await
        .ok()
        .flatten();
        let pressed = match play {
            Some(name) => name,
            None => {
                let o = crate::eyes::click_by_sight(ctx, &w, "the round green Play button on the Top result card", None).await;
                if o.status == crate::Status::Failed {
                    return Outcome::unsure(format!("opened Spotify's search for \"{query}\" but couldn't find its Play button"));
                }
                "the top result".into()
            }
        };
        for _ in 0..12 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let sessions = tokio::task::spawn_blocking(crate::platform::now_playing).await.ok().and_then(Result::ok).unwrap_or_default();
            if let Some(now) = sessions.iter().find(|n| n.playing && n.app.to_lowercase().contains("spotify")) {
                return Outcome::ok(format!("playing \"{}\" by {} in Spotify", now.title, now.artist));
            }
        }
        Outcome::unsure(format!("pressed {pressed} in Spotify; it hasn't shown up as playing yet"))
    }
    #[cfg(not(windows))]
    Outcome::failed("not on this OS yet")
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_results() {
        let html = r#"..."videoRenderer":{"videoId":"4NRXx6U8ABQ","thumbnail":{},"title":{"runs":[{"text":"The Weeknd - Blinding Lights (Official Audio)"}]}..."reelItemRenderer":{"videoId":"shortshort1"}..."videoRenderer":{"videoId":"fHI8X4OXluQ","title":{"runs":[{"text":"Blinding Lights & more"}]}"#;
        let v = super::videos(html);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], ("4NRXx6U8ABQ".to_string(), "The Weeknd - Blinding Lights (Official Audio)".to_string()));
        assert_eq!(v[1].1, "Blinding Lights & more");
    }
}
