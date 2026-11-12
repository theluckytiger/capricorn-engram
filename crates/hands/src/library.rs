//! Games (and anything else you keep a shortcut to) that the Start menu may not list, from where each store keeps its
//! own list, all read on this PC: Steam (`libraryfolders.vdf`, `appmanifest_*.acf`), Epic (its `.item` manifests),
//! GOG and Ubisoft (the registry), and the shortcuts on the desktop. Xbox / Game Pass games are Store apps, which the
//! Start menu has already. Anything still missing, she finds on screen, in the launcher's own window.
//!
//! Games are also known by other names: "start Warzone" means Steam's "Call of Duty®".

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Something to start, and how: a URL a launcher handles (`steam://rungameid/…`) or a file to open (a shortcut).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub launch: String,
    /// Where it was found: "Steam", "Epic", "GOG", "Ubisoft", "desktop".
    pub from: &'static str,
}

/// Other names for games, as people say them, and the name their store knows them by.
const ALIASES: &[(&str, &str)] = &[
    ("warzone", "Call of Duty"),
    ("cod", "Call of Duty"),
    ("call of duty warzone", "Call of Duty"),
    ("black ops", "Call of Duty"),
    ("modern warfare", "Call of Duty"),
    ("mw", "Call of Duty"),
    ("rivals", "Marvel Rivals"),
    ("cs", "Counter-Strike 2"),
    ("cs2", "Counter-Strike 2"),
    ("counter strike", "Counter-Strike 2"),
    ("dota", "Dota 2"),
    ("gta", "Grand Theft Auto V"),
    ("gta 5", "Grand Theft Auto V"),
    ("pubg", "PUBG: BATTLEGROUNDS"),
    ("cyberpunk", "Cyberpunk 2077"),
    ("fortnite", "Fortnite"),
    ("val", "VALORANT"),
    ("lol", "League of Legends"),
];

/// A store's name for a game said another way ("war zone" → "Call of Duty"), or the name as it was.
pub fn alias(name: &str) -> String {
    let n = crate::matching::norm(name);
    let squashed: String = n.chars().filter(|c| !c.is_whitespace()).collect();
    ALIASES
        .iter()
        .find(|(a, _)| *a == n || a.replace(' ', "") == squashed)
        .map(|(_, real)| real.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Store tools and runtimes, which have manifests but aren't games.
fn tool(name: &str) -> bool {
    let n = name.to_lowercase();
    ["redistributables", "proton", "steam linux runtime", "steamvr", "sdk", "dedicated server"].iter().any(|t| n.contains(t))
}

/// Everything found, from every store and the desktop.
pub fn entries() -> Vec<Entry> {
    let mut out = steam();
    out.extend(epic());
    #[cfg(windows)]
    {
        out.extend(gog());
        out.extend(ubisoft());
    }
    out.extend(desktop());
    out
}

// --- Steam

fn steam() -> Vec<Entry> {
    let mut candidates = vec![PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"C:\Program Files\Steam")];
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(Path::new(&home).join(".steam/steam"));
        candidates.push(Path::new(&home).join(".local/share/Steam"));
    }
    let mut out = Vec::new();
    for steam in candidates.into_iter().filter(|p| p.join("steamapps").is_dir()) {
        let mut libraries = vec![steam.clone()];
        if let Ok(vdf) = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
            libraries.extend(values(&vdf, "path").into_iter().map(|p| PathBuf::from(p.replace("\\\\", "\\"))));
        }
        libraries.dedup();
        for lib in libraries {
            let Ok(dir) = std::fs::read_dir(lib.join("steamapps")) else { continue };
            for acf in dir.filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("appmanifest_")).filter_map(|e| std::fs::read_to_string(e.path()).ok()) {
                if let Some(e) = steam_manifest(&acf)
                    && !out.iter().any(|x: &Entry| x.launch == e.launch)
                {
                    out.push(e);
                }
            }
        }
    }
    out
}

fn steam_manifest(acf: &str) -> Option<Entry> {
    let id = values(acf, "appid").into_iter().next()?;
    let name = values(acf, "name").into_iter().next().filter(|n| !tool(n))?;
    Some(Entry { name, launch: format!("steam://rungameid/{id}"), from: "Steam" })
}

/// Every value of `"key"  "value"` in Valve's text format.
fn values(text: &str, key: &str) -> Vec<String> {
    let quoted = format!("\"{key}\"");
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix(&quoted)?.trim();
            Some(rest.strip_prefix('"')?.strip_suffix('"')?.to_string())
        })
        .collect()
}

// --- Epic

fn epic() -> Vec<Entry> {
    let dir = PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join(r"Epic\EpicGamesLauncher\Data\Manifests");
    let Ok(dir) = std::fs::read_dir(dir) else { return vec![] };
    dir.filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "item"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|s| epic_manifest(&s))
        .collect()
}

fn epic_manifest(json: &str) -> Option<Entry> {
    let v: Value = serde_json::from_str(json).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let (name, ns, item, app) = (s("DisplayName")?, s("CatalogNamespace")?, s("CatalogItemId")?, s("AppName")?);
    Some(Entry { name: name.into(), launch: format!("com.epicgames.launcher://apps/{ns}%3A{item}%3A{app}?action=launch&silent=true"), from: "Epic" })
}

// --- GOG and Ubisoft (the registry)

#[cfg(windows)]
fn gog() -> Vec<Entry> {
    use crate::platform::registry;
    const KEY: &str = r"SOFTWARE\WOW6432Node\GOG.com\Games";
    registry::subkeys(KEY)
        .into_iter()
        .filter_map(|id| {
            let k = format!(r"{KEY}\{id}");
            let name = registry::value(&k, "gameName")?;
            let exe = registry::value(&k, "exe")?;
            Some(Entry { name, launch: exe, from: "GOG" })
        })
        .collect()
}

#[cfg(windows)]
fn ubisoft() -> Vec<Entry> {
    use crate::platform::registry;
    const KEY: &str = r"SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs";
    registry::subkeys(KEY)
        .into_iter()
        .filter_map(|id| {
            let dir = registry::value(&format!(r"{KEY}\{id}"), "InstallDir")?;
            let name = Path::new(dir.trim_end_matches(['/', '\\'])).file_name()?.to_string_lossy().into_owned();
            Some(Entry { name, launch: format!("uplay://launch/{id}/0"), from: "Ubisoft" })
        })
        .collect()
}

// --- the desktop

/// Shortcuts on the desktop (yours and everyone's): launchers like EA's and Battle.net's put their games there.
fn desktop() -> Vec<Entry> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("USERPROFILE") {
        dirs.push(Path::new(&home).join("Desktop"));
        dirs.push(Path::new(&home).join(r"OneDrive\Desktop"));
    }
    dirs.push(PathBuf::from(std::env::var("PUBLIC").unwrap_or_else(|_| r"C:\Users\Public".into())).join("Desktop"));
    dirs.iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk") || x.eq_ignore_ascii_case("url")))
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_stem()?.to_string_lossy().into_owned();
            Some(Entry { name, launch: path.to_string_lossy().into_owned(), from: "desktop" })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_steam_files() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"1938090\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"Call of Duty®\"\n\t\"installdir\"\t\t\"Call of Duty HQ\"\n}";
        assert_eq!(steam_manifest(acf), Some(Entry { name: "Call of Duty®".into(), launch: "steam://rungameid/1938090".into(), from: "Steam" }));
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(values(vdf, "path"), vec!["C:\\\\Program Files (x86)\\\\Steam", "D:\\\\SteamLibrary"]);
        assert!(steam_manifest("\"appid\" \"228980\"\n\"name\" \"Steamworks Common Redistributables\"").is_none());
    }

    #[test]
    fn reads_an_epic_manifest() {
        let item = r#"{"DisplayName": "Fortnite", "AppName": "Fortnite", "CatalogNamespace": "fn", "CatalogItemId": "4fe75bbc5a674f4f9b356b5c90567da5"}"#;
        let e = epic_manifest(item).unwrap();
        assert_eq!(e.name, "Fortnite");
        assert_eq!(e.launch, "com.epicgames.launcher://apps/fn%3A4fe75bbc5a674f4f9b356b5c90567da5%3AFortnite?action=launch&silent=true");
    }

    #[test]
    fn other_names() {
        assert_eq!(alias("Warzone"), "Call of Duty");
        assert_eq!(alias("war zone"), "Call of Duty");
        assert_eq!(alias("Marvel Rivals"), "Marvel Rivals");
    }
}
