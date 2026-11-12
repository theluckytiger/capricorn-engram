//! Matching what you said ("discord", "the brave window", "steam") to names on your PC, by how surely it's that one.
//! She acts alone only on a sure match; otherwise the candidates go back to her brain to choose or ask.

/// How surely `name` is what `query` meant: 0 (no) to 100 (exactly).
pub fn score(query: &str, name: &str) -> u32 {
    let q = norm(query);
    let n = norm(name);
    if q.is_empty() || n.is_empty() {
        return 0;
    }
    let qw: Vec<&str> = q.split(' ').collect();
    let nw: Vec<&str> = n.split(' ').collect();
    let mut s = if n == q {
        100
    } else if nw.starts_with(&qw) {
        // "spotify" ~ "spotify premium", "brave" ~ "brave browser"
        88 - (nw.len() - qw.len()).min(4) as u32 * 3
    } else if qw.iter().all(|w| nw.contains(w)) {
        72
    } else if qw.iter().all(|w| nw.iter().any(|x| x.starts_with(w) && w.len() >= 3)) {
        62
    } else if n.replace(' ', "") == q.replace(' ', "") {
        // "vs code" ~ "vscode"
        90
    } else if sounds(&n.replace(' ', "")) == sounds(&q.replace(' ', "")) {
        // A name as her ears wrote it: "Stremy O" ~ "Stremio".
        80
    } else if qw.len() == 1 && nw.iter().any(|x| close(x, qw[0])) {
        // one typo: "discrod" ~ "discord"
        50
    } else {
        0
    };
    // Helpers that only share a word with the app are rarely what you meant.
    if s > 0 && !qw.iter().any(|w| JUNK.contains(w)) && nw.iter().any(|w| JUNK.contains(w)) {
        s = s.saturating_sub(40);
    }
    s
}

const JUNK: &[&str] = &["uninstall", "uninstaller", "help", "readme", "website", "documentation", "manual", "release", "notes", "license", "updater", "update", "repair", "safe", "mode"];

pub fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = true;
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() {
            out.push(c);
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out.trim_end().to_string()
}

/// How a name sounds, roughly: y like i, ph like f, k like c.
fn sounds(s: &str) -> String {
    s.replace("ph", "f").replace('y', "i").replace('k', "c")
}

/// Edit distance at most 1, for words of 4 letters or more.
fn close(a: &str, b: &str) -> bool {
    if a.len() < 4 || b.len() < 4 || a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if a.len() > b.len() {
            i += 1;
        } else if b.len() > a.len() {
            j += 1;
        } else if i + 1 < a.len() && j + 1 < b.len() && a[i] == b[j + 1] && a[i + 1] == b[j] {
            // a swap of two neighbours counts as one
            i += 2;
            j += 2;
        } else {
            i += 1;
            j += 1;
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}

/// The best candidates, best first, with their scores; only those scoring at least `min`.
pub fn rank<'a, T>(query: &str, items: &'a [T], names: impl Fn(&T) -> Vec<&str>, min: u32) -> Vec<(u32, &'a T)> {
    let mut out: Vec<(u32, &T)> = items.iter().map(|it| (names(it).into_iter().map(|n| score(query, n)).max().unwrap_or(0), it)).filter(|(s, _)| *s >= min).collect();
    out.sort_by_key(|x| std::cmp::Reverse(x.0));
    out
}

/// When nothing matched: the name closest to what was said, if it's close enough to ask "did you mean…?" about
/// ("Streo" → "Stremio": her ears drop a letter or two). Compared without spaces, as it sounds, whole or word by word.
pub fn closest<'a, T>(query: &str, items: &'a [T], names: impl Fn(&T) -> Vec<&str>) -> Option<&'a T> {
    let q = sounds(&norm(query).replace(' ', ""));
    if q.chars().count() < 3 {
        return None;
    }
    let like = |name: &str| -> f32 {
        let n = norm(name);
        std::iter::once(n.replace(' ', "")).chain(n.split(' ').map(str::to_string)).filter(|p| p.len() >= 3).map(|p| {
            let p = sounds(&p);
            1.0 - levenshtein(&q, &p) as f32 / q.chars().count().max(p.chars().count()) as f32
        }).fold(0.0, f32::max)
    };
    items.iter().map(|it| (names(it).into_iter().map(like).fold(0.0, f32::max), it)).filter(|(s, _)| *s >= 0.6).max_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, it)| it)
}

/// Letters to change, add or drop to turn `a` into `b`.
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (diag + usize::from(ca != *cb)).min(row[j] + 1).min(row[j + 1] + 1);
            diag = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// A match is sure when it scores high and nothing else is close behind.
pub fn sure<T>(ranked: &[(u32, T)]) -> bool {
    match ranked {
        [(a, _)] => *a >= 62,
        [(a, _), (b, _), ..] => *a >= 72 && a - b >= 10,
        [] => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_name_to_ask_about() {
        let apps = ["Stremio", "Steam", "Spotify", "Discord"];
        assert_eq!(super::closest("Streo", &apps, |a| vec![*a]), Some(&"Stremio"));
        assert_eq!(super::closest("Discrd", &apps, |a| vec![*a]), Some(&"Discord"));
        assert_eq!(super::closest("Photoshop", &apps, |a| vec![*a]), None);
    }

    #[test]
    fn names_as_heard() {
        assert_eq!(super::score("Stremy O", "Stremio"), 80);
        assert_eq!(super::score("discord", "Discord"), 100);
    }

    #[test]
    fn exact_beats_helpers() {
        assert!(score("discord", "Discord") > score("discord", "Uninstall Discord"));
        assert!(score("steam", "Steam") > score("steam", "Steam Support Center"));
        assert_eq!(score("discord", "Spotify"), 0);
    }

    #[test]
    fn prefixes_and_typos() {
        assert!(score("brave", "Brave Browser") >= 80);
        assert!(score("vs code", "VSCode") >= 88);
        assert!(score("discrod", "Discord") > 0);
        assert!(score("obs", "OBS Studio") >= 80);
    }

    #[test]
    fn sure_needs_a_gap() {
        let names = ["Discord", "Discord PTB"];
        let r = rank("discord", &names, |n| vec![*n], 1);
        assert!(sure(&r), "{r:?}");
        let names = ["Minecraft Launcher", "Minecraft Dungeons"];
        let r = rank("minecraft", &names, |n| vec![*n], 1);
        assert!(!sure(&r), "{r:?}");
    }
}
