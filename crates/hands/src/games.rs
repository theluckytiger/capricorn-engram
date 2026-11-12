//! Telling a game from everything else, by what it does: its window fills a screen and it works the GPU's 3D
//! engine, for a few seconds running. Where it was installed from only lowers the bar (a Steam folder), it's never
//! required: a cracked exe, an emulator or a game you built yourself all count.

use std::collections::VecDeque;

/// One look at the program in front.
#[derive(Debug, Clone)]
pub struct Look {
    pub pid: u32,
    pub exe: String,
    /// Its full path, lowercase, for the launcher hint.
    pub path: String,
    pub title: String,
    pub fullscreen: bool,
    /// Percent of the GPU's 3D engines it used since the last look.
    pub gpu: f64,
}

/// Never games, however hard they work the GPU.
const NOT_GAMES: &[&str] = &[
    "brave", "chrome", "msedge", "firefox", "opera", "vivaldi", "arc", "vlc", "mpc-hc64", "mpc-be64", "mpv", "potplayermini64", "wmplayer", "obs64", "obs", "discord", "explorer", "applicationframehost", "teams", "ms-teams", "zoom", "code",
    "devenv", "idea64", "pycharm64", "rustrover64", "blender", "photoshop", "afterfx", "premiere", "resolve", "unity", "unrealeditor", "claude", "spotify", "steamwebhelper", "epicgameslauncher", "capricorn", "dwm", "searchhost",
    "startmenuexperiencehost", "shellexperiencehost", "lockapp", "nvidia app", "nvcontainer", "msedgewebview2", "powerpnt", "winword", "excel", "wallpaper64", "wallpaper32", "lively", "rainmeter",
];

const LAUNCHER_HINTS: &[&str] = &["\\steamapps\\common\\", "\\epic games\\", "\\gog galaxy\\games\\", "\\gog games\\", "\\xboxgames\\", "\\riot games\\", "\\ea games\\", "\\ubisoft game launcher\\games\\", "\\battle.net\\"];

/// What a game is remembered by (quiet or reachable): its folder in a store's library when it has one, since a
/// launcher and the game it starts share it (Call of Duty's `cod.exe` and `cod25-cod.exe`, both under
/// `steamapps\common\Call of Duty HQ`: she asked twice), else its exe.
pub fn family(exe: &str, path: &str) -> String {
    let p = path.to_lowercase().replace('/', "\\");
    for hint in LAUNCHER_HINTS {
        if let Some(i) = p.find(hint)
            && let Some(folder) = p[i + hint.len()..].split('\\').next().filter(|f| !f.is_empty() && !f.ends_with(".exe"))
        {
            return format!("game:{folder}");
        }
    }
    exe.to_string()
}

/// Watches the program in front over time and says when it's a game.
#[derive(Default)]
pub struct Detector {
    recent: VecDeque<Look>,
}

impl Detector {
    /// Feed one look (every 2 s or so). Returns the game when the last looks say it's one.
    pub fn feed(&mut self, look: Look) -> Option<Look> {
        if self.recent.back().is_some_and(|l| l.pid != look.pid) {
            self.recent.clear();
        }
        self.recent.push_back(look);
        while self.recent.len() > 3 {
            self.recent.pop_front();
        }
        let last = self.recent.back()?.clone();
        if NOT_GAMES.contains(&last.exe.as_str()) {
            return None;
        }
        let launcher = LAUNCHER_HINTS.iter().any(|h| last.path.contains(h));
        let looks = self.recent.len();
        let all = |f: &dyn Fn(&Look) -> bool| self.recent.iter().all(f);
        let game = if launcher {
            // Installed as a game: working the GPU at all, twice in a row.
            looks >= 2 && all(&|l| l.gpu >= 8.0)
        } else {
            // Anything else: fullscreen and working the GPU, three looks running (~6 s).
            looks >= 3 && all(&|l| l.fullscreen && l.gpu >= 15.0)
        };
        game.then_some(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launcher_and_its_game_are_one() {
        let launcher = family("cod", r"d:\steamlibrary\steamapps\common\call of duty hq\cod.exe");
        let game = family("cod25-cod", r"d:\steamlibrary\steamapps\common\call of duty hq\cod25\cod25-cod.exe");
        assert_eq!(launcher, game);
        assert_eq!(launcher, "game:call of duty hq");
        assert_eq!(family("mygame", r"c:\games\mygame\mygame.exe"), "mygame");
    }

    fn look(exe: &str, path: &str, fullscreen: bool, gpu: f64) -> Look {
        Look { pid: 7, exe: exe.into(), path: path.into(), title: String::new(), fullscreen, gpu }
    }

    #[test]
    fn a_cracked_fullscreen_game_counts() {
        let mut d = Detector::default();
        let l = || look("eldenring", "d:\\games\\elden ring\\game\\eldenring.exe", true, 72.0);
        assert!(d.feed(l()).is_none());
        assert!(d.feed(l()).is_none());
        assert!(d.feed(l()).is_some());
    }

    #[test]
    fn a_steam_game_counts_sooner_even_windowed() {
        let mut d = Detector::default();
        let l = || look("brawlhalla", "c:\\program files (x86)\\steam\\steamapps\\common\\brawlhalla\\brawlhalla.exe", false, 20.0);
        assert!(d.feed(l()).is_none());
        assert!(d.feed(l()).is_some());
    }

    #[test]
    fn fullscreen_youtube_isnt_a_game() {
        let mut d = Detector::default();
        for _ in 0..5 {
            assert!(d.feed(look("brave", "c:\\program files\\bravesoftware\\brave-browser\\application\\brave.exe", true, 40.0)).is_none());
        }
    }

    #[test]
    fn a_windowed_editor_isnt_a_game() {
        let mut d = Detector::default();
        for _ in 0..5 {
            assert!(d.feed(look("notepad", "c:\\windows\\notepad.exe", false, 30.0)).is_none());
        }
    }
}
