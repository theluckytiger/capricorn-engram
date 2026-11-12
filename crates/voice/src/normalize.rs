//! Text for voices that only say words from a pronunciation lexicon (Kokoro, Kitten): without this, anything not in
//! it — numbers, "screen's", "Spotify" — is silently skipped. Every word that comes out of here is one the lexicon
//! knows (or a letter, which it always knows).
//!
//! Words it doesn't know are learned ([`learn`]): a form of one it knows ("opened" from "open", "that's", "vids",
//! "glitchin'") gets that word's sounds plus the ending's; a name ("Zorin", "Stremio") gets English spelling rules.
//! They're kept in her own word list, which her voice loads beside the lexicon. (Kokoro's lexicon has 178k words
//! and still no "opened" or "that's": she said "I open e d".)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Lexicon {
    words: HashSet<Box<str>>,
    /// Where the words came from (their sounds are read from these when a new word is learned from one).
    paths: Vec<PathBuf>,
    /// The sounds the voice has (tokens.txt): a learned word may use only these.
    sounds: HashSet<Box<str>>,
}

impl Lexicon {
    /// Loads the words (first column) of sherpa-onnx lexicon files (missing ones are skipped: her own word list
    /// may not exist yet), and the voice's sounds from `tokens`.
    pub fn load(paths: &[&Path], tokens: Option<&Path>) -> std::io::Result<Self> {
        let mut words = HashSet::new();
        for p in paths {
            let Ok(text) = std::fs::read_to_string(p) else { continue };
            for line in text.lines() {
                if let Some(w) = line.split_whitespace().next() {
                    words.insert(w.to_lowercase().into_boxed_str());
                }
            }
        }
        if words.is_empty() {
            return Err(std::io::Error::other("no lexicon"));
        }
        let sounds = tokens.and_then(|t| std::fs::read_to_string(t).ok()).map(|t| t.lines().filter_map(|l| l.split(' ').next()).filter(|s| !s.is_empty()).map(Into::into).collect()).unwrap_or_default();
        Ok(Self { words, paths: paths.iter().map(|p| p.to_path_buf()).collect(), sounds })
    }

    pub fn from_words(words: &[&str]) -> Self {
        Self { words: words.iter().map(|w| w.to_lowercase().into_boxed_str()).collect(), paths: Vec::new(), sounds: HashSet::new() }
    }

    pub fn has(&self, w: &str) -> bool {
        self.words.contains(w)
    }

    /// A word she's just learned.
    pub fn add(&mut self, w: &str) {
        self.words.insert(w.into());
    }

    /// The sounds of `words`, read from the lexicon files (only when learning: they aren't kept in memory).
    fn sounds_of(&self, words: &HashSet<String>) -> HashMap<String, String> {
        let mut found = HashMap::new();
        for p in &self.paths {
            let Ok(text) = std::fs::read_to_string(p) else { continue };
            for line in text.lines() {
                if let Some((w, s)) = line.split_once(' ')
                    && words.contains(w)
                    && !found.contains_key(w)
                {
                    found.insert(w.to_string(), s.trim().to_string());
                }
            }
        }
        found
    }
}

const ONES: &[&str] = &["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
const TENS: &[&str] = &["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

/// 0..=999_999_999_999 in words: 32 → "thirty two", 1500 → "one thousand five hundred".
pub fn number(n: u64) -> String {
    fn under_1000(n: u64) -> String {
        let mut parts = Vec::new();
        if n >= 100 {
            parts.push(format!("{} hundred", ONES[(n / 100) as usize]));
        }
        let r = n % 100;
        if r >= 20 {
            parts.push(if r.is_multiple_of(10) { TENS[(r / 10) as usize].to_string() } else { format!("{} {}", TENS[(r / 10) as usize], ONES[(r % 10) as usize]) });
        } else if r > 0 || n == 0 {
            parts.push(ONES[r as usize].to_string());
        }
        parts.join(" ")
    }
    if n < 1000 {
        return under_1000(n);
    }
    let mut parts = Vec::new();
    for (size, name) in [(1_000_000_000, "billion"), (1_000_000, "million"), (1_000, "thousand")] {
        if n >= size && !(n / size).is_multiple_of(1000) {
            parts.push(format!("{} {name}", under_1000((n / size) % 1000)));
        }
    }
    if !n.is_multiple_of(1000) {
        parts.push(under_1000(n % 1000));
    }
    parts.join(" ")
}

/// "04:00" → "four", "16:30" → "sixteen thirty", "9:05" → "nine oh five".
fn time(h: u64, m: u64) -> String {
    match m {
        0 => number(h),
        1..=9 => format!("{} oh {}", number(h), number(m)),
        _ => format!("{} {}", number(h), number(m)),
    }
}

/// Symbols and number shapes to words, before the words are checked.
fn spell_out(text: &str) -> String {
    // Dashes between words are pauses, not part of a word ("tomorrow—don't" was read "don t"): a comma, which her
    // voice pauses on (a dash on its own it skipped). Markdown and quote marks stuck to a word made her voice drop the
    // word ("**3,474 km**", "\"Shade\""): gone.
    let t = text
        .replace(['—', '–'], ", ")
        .replace(['\u{2010}', '\u{2011}'], "-")
        .replace(['’', '‘', 'ʼ'], "'")
        .replace(['“', '”', '"', '*', '_', '#', '`', '~', '|', '[', ']', '{', '}', '<', '>'], " ")
        .replace("°C", " degrees").replace("°F", " degrees").replace('°', " degrees").replace('%', " percent").replace('&', " and ").replace(" km/h", " kilometers an hour").replace("km/h", " kilometers an hour");
    let chars: Vec<char> = t.chars().collect();
    let mut out = String::with_capacity(t.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let a: String = chars[start..i].iter().collect();
        let mut a_n: u64 = a.parse().unwrap_or(0);
        // 3,474 and 1,500,000: thousands, not a list ("three, four hundred seventy four").
        while chars.get(i) == Some(&',')
            && chars.get(i + 1..i + 4).is_some_and(|g| g.iter().all(char::is_ascii_digit))
            && !chars.get(i + 4).is_some_and(char::is_ascii_digit)
        {
            a_n = a_n * 1000 + chars[i + 1..i + 4].iter().collect::<String>().parse::<u64>().unwrap_or(0);
            i += 4;
        }
        // hh:mm
        if i + 2 < chars.len() && chars[i] == ':' && chars[i + 1].is_ascii_digit() && chars[i + 2].is_ascii_digit() {
            let m: u64 = chars[i + 1..i + 3].iter().collect::<String>().parse().unwrap_or(0);
            out.push_str(&time(a_n, m));
            i += 3;
            continue;
        }
        // 3.5
        if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
            let s = i + 1;
            let mut j = s;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            let digits: Vec<String> = chars[s..j].iter().map(|d| ONES[d.to_digit(10).unwrap() as usize].to_string()).collect();
            out.push_str(&format!("{} point {}", number(a_n), digits.join(" ")));
            i = j;
            continue;
        }
        out.push_str(&number(a_n));
        // a unit stuck to the number: "6mm", "48h"
        let unit: String = chars[i..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
        let word = match unit.as_str() {
            "mm" => Some(" millimeters"),
            "cm" => Some(" centimeters"),
            "km" => Some(" kilometers"),
            "h" => Some(" hours"),
            "s" => Some(" seconds"),
            "gb" | "GB" => Some(" gigabytes"),
            "mb" | "MB" => Some(" megabytes"),
            "st" | "nd" | "rd" | "th" => Some(""),
            _ => None,
        };
        if let Some(w) = word {
            out.push_str(w);
            i += unit.chars().count();
        }
        out.push(' ');
    }
    // A unit after a space ("4.8 mm"), wherever it falls, the end of the text included.
    let mut out = format!("{out} ");
    for (unit, word) in [(" mm", " millimeters"), (" km", " kilometers"), (" cm", " centimeters")] {
        for end in [" ", ".", ","] {
            out = out.replace(&format!("{unit}{end}"), &format!("{word}{end}"));
        }
    }
    out.trim_end().to_string()
}

/// The text, with every word one the lexicon can say.
pub fn for_lexicon(text: &str, lex: &Lexicon) -> String {
    let spelled = spell_out(text);
    let mut out: Vec<String> = Vec::new();
    for token in spelled.split_whitespace() {
        // Split leading/trailing punctuation off; it carries the pauses.
        let core_start = token.find(|c: char| c.is_alphanumeric()).unwrap_or(token.len());
        let core_end = token.rfind(|c: char| c.is_alphanumeric()).map(|e| e + token[e..].chars().next().unwrap().len_utf8()).unwrap_or(core_start);
        let (pre, core, post) = (&token[..core_start], &token[core_start..core_end.max(core_start)], &token[core_end.max(core_start)..]);
        if core.is_empty() {
            out.push(token.to_string());
            continue;
        }
        let said = say_word(core, lex);
        // An apostrophe that only drops a letter ("missin'") isn't part of the word her voice looks up.
        out.push(format!("{}{said}{}", pre.trim_end_matches('\''), post.trim_start_matches('\'')));
    }
    out.join(" ")
}

/// The words in `text` her voice doesn't know, and their sounds, worked out: to be added to her word list.
pub fn learn(text: &str, lex: &Lexicon) -> Vec<(String, String)> {
    let mut wanted = Vec::new();
    for token in spell_out(text).split_whitespace() {
        let core: &str = token.trim_matches(|c: char| !c.is_alphanumeric());
        if !core.is_empty() {
            unknown(core, lex, &mut wanted);
        }
    }
    let mut seen = HashSet::new();
    wanted.retain(|w| seen.insert(w.clone()));
    if wanted.is_empty() {
        return Vec::new();
    }
    // The words they could be forms of, looked up all at once.
    let bases: HashSet<String> = wanted.iter().flat_map(|w| forms(w).into_iter().map(|(b, _)| b)).filter(|b| lex.has(b)).collect();
    let known = lex.sounds_of(&bases);
    wanted
        .into_iter()
        .filter_map(|w| {
            let from_base = forms(&w).into_iter().find_map(|(b, end)| known.get(&b).map(|s| ending(s, end)));
            let sounds = from_base.or_else(|| spell(&w))?;
            // Only sounds her voice has (else the whole word list would fail to load).
            let ok = !lex.sounds.is_empty() && sounds.split(' ').all(|s| lex.sounds.contains(s));
            ok.then_some((w, sounds))
        })
        .collect()
}

/// The words `say_word` would have to chop up: what's worth learning.
fn unknown(word: &str, lex: &Lexicon, out: &mut Vec<String>) {
    let w = word.to_lowercase();
    if lex.has(&w) {
        return;
    }
    if let Some(base) = w.strip_suffix("'s") {
        if sounds_alike(base, lex).is_some() {
            return;
        }
        if lex.has(base) {
            out.push(w);
            return;
        }
    }
    if w.contains(['-', '/', '_', '.']) {
        w.split(['-', '/', '_', '.']).filter(|p| !p.is_empty()).for_each(|p| unknown(p, lex, out));
        return;
    }
    // CamelCase ("YouTubeMusic"): its parts.
    let caps = word.chars().skip(1).any(|c| c.is_uppercase()) && word.chars().any(|c| c.is_lowercase());
    if caps {
        return;
    }
    // An abbreviation, said letter by letter (GPU).
    if word.len() <= 5 && word.chars().all(|c| c.is_ascii_uppercase()) {
        return;
    }
    if w.len() >= 2 && w.chars().all(|c| c.is_ascii_alphabetic() || c == '\'') {
        out.push(w);
    }
}

/// What a word could be a form of: the base word, and the ending it has.
#[derive(Clone, Copy, PartialEq, Debug)]
enum End {
    S,
    Ed,
    Ing,
    In,
    /// "comin'" from "coming": its -ing said -in.
    InFromIng,
}

fn forms(w: &str) -> Vec<(String, End)> {
    let mut f = Vec::new();
    let mut add = |b: &str, e: End| {
        if b.len() >= 2 {
            f.push((b.to_string(), e));
        }
    };
    if let Some(b) = w.strip_suffix("'s") {
        add(b, End::S);
    }
    if let Some(b) = w.strip_suffix("ies") {
        add(&format!("{b}y"), End::S);
    }
    if let Some(b) = w.strip_suffix("es") {
        add(b, End::S);
    }
    if let Some(b) = w.strip_suffix('s') {
        add(b, End::S);
    }
    if let Some(b) = w.strip_suffix("ied") {
        add(&format!("{b}y"), End::Ed);
    }
    if let Some(b) = w.strip_suffix("ed") {
        add(b, End::Ed);
        add(&format!("{b}e"), End::Ed);
        // stopped → stop
        if b.len() >= 3 && b.as_bytes()[b.len() - 1] == b.as_bytes()[b.len() - 2] {
            add(&b[..b.len() - 1], End::Ed);
        }
    }
    if let Some(b) = w.strip_suffix('d') {
        add(b, End::Ed);
    }
    if let Some(b) = w.strip_suffix("in'").or_else(|| w.strip_suffix("in")) {
        add(&format!("{b}ing"), End::InFromIng);
    }
    for (suffix, end) in [("ing", End::Ing), ("in'", End::In), ("in", End::In)] {
        if let Some(b) = w.strip_suffix(suffix) {
            // "making" from "make" before "mak": a dropped e is the likelier spelling.
            add(&format!("{b}e"), end);
            add(b, end);
            if b.len() >= 3 && b.as_bytes()[b.len() - 1] == b.as_bytes()[b.len() - 2] {
                add(&b[..b.len() - 1], end);
            }
        }
    }
    f
}

/// A word's sounds with an ending: "-s" is s after a voiceless sound, ᵻz after a hiss, z otherwise; "-ed" is ᵻd
/// after t or d, t after a voiceless sound, d otherwise.
fn ending(sounds: &str, end: End) -> String {
    let last = sounds.split(' ').rev().find(|s| *s != "ˈ" && *s != "ˌ" && *s != "ː").unwrap_or("");
    let voiceless = matches!(last, "p" | "t" | "k" | "f" | "θ" | "s" | "ʃ" | "ʧ");
    let add = match end {
        End::S if matches!(last, "s" | "z" | "ʃ" | "ʒ" | "ʧ" | "ʤ") => "ᵻ z",
        End::S if voiceless => "s",
        End::S => "z",
        End::Ed if matches!(last, "t" | "d") => "ᵻ d",
        End::Ed if voiceless => "t",
        End::Ed => "d",
        End::Ing => "ɪ ŋ",
        End::In => "ɪ n",
        End::InFromIng => return sounds.strip_suffix('ŋ').map(|s| format!("{s}n")).unwrap_or_else(|| sounds.to_string()),
    };
    format!("{sounds} {add}")
}

/// A name or new word, by English spelling rules ("zorin" → z ˈ ɑ ɹ ɪ n, "stremio" → s t ɹ ˈ ɛ m i O): rough, but
/// a word, not letters.
fn spell(w: &str) -> Option<String> {
    let c: Vec<char> = w.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if c.len() < 2 {
        return None;
    }
    let vowel = |i: usize| c.get(i).is_some_and(|ch| "aeiou".contains(*ch));
    let mut out: Vec<&str> = Vec::new();
    let mut stressed = false;
    let mut i = 0;
    // A vowel sound: the first one is stressed.
    let mut v = |out: &mut Vec<&str>, s: &'static str, weak: &'static str| {
        if stressed {
            out.push(weak);
        } else {
            out.push("ˈ");
            out.push(s);
            stressed = true;
        }
    };
    while i < c.len() {
        let rest: String = c[i..].iter().collect();
        let at_end = |n: usize| i + n >= c.len();
        // Two- and three-letter spellings first.
        let two: Option<(usize, &str)> = [("tch", "ʧ"), ("sch", "s k"), ("ch", "ʧ"), ("sh", "ʃ"), ("th", "θ"), ("ph", "f"), ("ck", "k"), ("qu", "k w"), ("wh", "w"), ("gh", "")]
            .iter()
            .find(|(p, _)| rest.starts_with(p))
            .map(|(p, s)| (p.len(), *s));
        if let Some((n, s)) = two {
            out.extend(s.split(' ').filter(|x| !x.is_empty()));
            i += n;
            continue;
        }
        let vowels: Option<(usize, &'static str)> = [("ee", "i"), ("ea", "i"), ("ie", "i"), ("oo", "u"), ("ou", "W"), ("ow", "O"), ("oi", "Y"), ("oy", "Y"), ("ai", "A"), ("ay", "A"), ("ei", "A"), ("ey", "A"), ("oa", "O"), ("au", "ɔ"), ("aw", "ɔ"), ("ue", "u"), ("ew", "u")]
            .iter()
            .find(|(p, _)| rest.starts_with(p))
            .map(|(p, s)| (p.len(), *s));
        if let Some((n, s)) = vowels {
            v(&mut out, s, s);
            i += n;
            continue;
        }
        let ch = c[i];
        // A doubled consonant is one sound.
        if i > 0 && c[i - 1] == ch && !vowel(i) {
            i += 1;
            continue;
        }
        match ch {
            'a' | 'e' | 'i' | 'o' | 'u' => {
                // A silent e at the end; a vowel, one consonant, then e: the long sound (Mike, Jade).
                if ch == 'e' && at_end(1) && i > 0 {
                    i += 1;
                    continue;
                }
                let long = c.get(i + 2) == Some(&'e') && at_end(3) && !vowel(i + 1);
                let (s, weak) = match ch {
                    'a' if long => ("A", "A"),
                    'a' if at_end(1) => ("ɑ", "ə"),
                    'a' => ("æ", "ə"),
                    'e' if long => ("i", "i"),
                    'e' => ("ɛ", "ɛ"),
                    'i' if long => ("I", "I"),
                    'i' if at_end(1) || vowel(i + 1) || c.get(i + 1) == Some(&'y') => ("i", "i"),
                    'i' => ("ɪ", "ɪ"),
                    'o' if long || at_end(1) => ("O", "O"),
                    'o' => ("ɑ", "ɑ"),
                    _ if long => ("u", "u"),
                    // A word starting u, consonant, vowel: "you" (Ubisoft, Uber).
                    _ if i == 0 && !vowel(1) && vowel(2) => ("u", "u"),
                    _ => ("ʌ", "ə"),
                };
                if ch == 'u' && i == 0 && !vowel(1) && vowel(2) {
                    out.push("j");
                }
                v(&mut out, s, weak);
                // r after a vowel: part of it.
                if c.get(i + 1) == Some(&'r') && !vowel(i + 2) {
                    out.push("ɹ");
                    i += 1;
                }
            }
            'y' if i == 0 || vowel(i + 1) => out.push("j"),
            'y' => v(&mut out, if at_end(1) { "i" } else { "ɪ" }, if at_end(1) { "i" } else { "ɪ" }),
            'c' if matches!(c.get(i + 1), Some('e' | 'i' | 'y')) => out.push("s"),
            'c' | 'k' | 'q' => out.push("k"),
            'g' if c.get(i + 1) == Some(&'e') && at_end(2) => out.push("ʤ"),
            'g' => out.push("ɡ"),
            'j' => out.push("ʤ"),
            'x' => out.extend(["k", "s"]),
            'r' => out.push("ɹ"),
            'n' if c.get(i + 1) == Some(&'g') && at_end(2) => {
                out.push("ŋ");
                i += 1;
            }
            'b' | 'd' | 'f' | 'h' | 'l' | 'm' | 'n' | 'p' | 's' | 't' | 'v' | 'w' | 'z' => out.push(match ch {
                'b' => "b",
                'd' => "d",
                'f' => "f",
                'h' => "h",
                'l' => "l",
                'm' => "m",
                'n' => "n",
                'p' => "p",
                's' => "s",
                't' => "t",
                'v' => "v",
                'w' => "w",
                _ => "z",
            }),
            _ => {}
        }
        i += 1;
    }
    stressed.then(|| out.join(" "))
}

/// The plural that sounds like `base`'s (currency's, currencies), when her voice knows it: no new word to learn.
fn sounds_alike(base: &str, lex: &Lexicon) -> Option<String> {
    let y = base.strip_suffix('y').filter(|b| b.ends_with(|c: char| !"aeiou".contains(c))).map(|b| format!("{b}ies"));
    let es = (base.ends_with('s') || base.ends_with('x') || base.ends_with('z') || base.ends_with("ch") || base.ends_with("sh")).then(|| format!("{base}es"));
    [Some(format!("{base}s")), y, es].into_iter().flatten().find(|p| lex.has(p))
}

fn say_word(word: &str, lex: &Lexicon) -> String {
    let w = word.to_lowercase();
    if lex.has(&w) {
        return word.to_string();
    }
    // screen's → screens (the same sound); it's / you're are in the lexicon as they are.
    if let Some(base) = w.strip_suffix("'s") {
        if let Some(same) = sounds_alike(base, lex) {
            return same;
        }
        if lex.has(base) {
            return base.to_string();
        }
    }
    if w.contains(['-', '\'', '/', '_', '.']) {
        return w.split(['-', '\'', '/', '_', '.']).filter(|p| !p.is_empty()).map(|p| say_word(p, lex)).collect::<Vec<_>>().join(" ");
    }
    // CamelCase: "YouTubeMusic" → "You Tube Music"
    let mut parts = Vec::new();
    let mut cur = String::new();
    for c in word.chars() {
        if c.is_uppercase() && !cur.is_empty() && cur.chars().last().is_some_and(|p| p.is_lowercase()) {
            parts.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    parts.push(cur);
    if parts.len() > 1 {
        return parts.iter().map(|p| say_word(p, lex)).collect::<Vec<_>>().join(" ");
    }
    // Short all-caps: an abbreviation, said letter by letter (GPU, CPU, OBS).
    if word.len() <= 5 && word.chars().all(|c| c.is_ascii_uppercase()) {
        return w.chars().map(String::from).collect::<Vec<_>>().join(" ");
    }
    // Otherwise the longest pieces the lexicon knows, letters for what's left: "spotify" → "spot if y".
    let chars: Vec<char> = w.chars().collect();
    let mut i = 0;
    let mut pieces = Vec::new();
    while i < chars.len() {
        let mut best = None;
        for j in (i + 2..=chars.len()).rev() {
            let piece: String = chars[i..j].iter().collect();
            if lex.has(&piece) {
                best = Some((j, piece));
                break;
            }
        }
        match best {
            Some((j, piece)) => {
                pieces.push(piece);
                i = j;
            }
            None => {
                pieces.push(chars[i].to_string());
                i += 1;
            }
        }
    }
    pieces.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex() -> Lexicon {
        Lexicon::from_words(&["thirty", "two", "four", "sixteen", "eight", "three", "point", "five", "millimeters", "your", "screens", "cozy", "volume", "at", "percent", "the", "rain", "starts", "around", "spot", "if", "you", "tube", "music", "degrees", "and", "s", "p", "y", "g", "c", "u", "is", "a", "in", "o", "t", "i", "f"])
    }

    #[test]
    fn numbers() {
        assert_eq!(number(32), "thirty two");
        assert_eq!(number(100), "one hundred");
        assert_eq!(number(1500), "one thousand five hundred");
        assert_eq!(number(0), "zero");
        assert_eq!(number(2026), "two thousand twenty six");
    }

    #[test]
    fn times_units_and_numbers_become_words() {
        let l = lex();
        assert_eq!(for_lexicon("Volume's at 32%.", &l), "volume at thirty two percent.");
        assert!(for_lexicon("Starts at 04:00 and 16:30.", &l).contains("four and sixteen thirty"));
        assert!(for_lexicon("8°C", &l).contains("eight degrees"));
        assert!(for_lexicon("3.5 mm", &l).contains("three point five millimeters"));
        let l = Lexicon::from_words(&["about", "one", "three", "four", "five", "seventy", "hundred", "thousand", "million", "kilometers", "or"]);
        assert_eq!(for_lexicon("about 3,474 km", &l), "about three thousand four hundred seventy four kilometers");
        assert_eq!(for_lexicon("1,500,000 or 1,5", &l), "one million five hundred thousand or one ,five");
    }

    #[test]
    fn new_words_are_learned_from_ones_she_knows() {
        assert_eq!(ending("ˈ O p ᵊ n", End::Ed), "ˈ O p ᵊ n d");
        assert_eq!(ending("w ˈ ɑ n t", End::Ed), "w ˈ ɑ n t ᵻ d");
        assert_eq!(ending("ð æ t", End::S), "ð æ t s");
        assert_eq!(ending("v ˈ ɪ d", End::S), "v ˈ ɪ d z");
        assert_eq!(ending("b ˈ ɑ k s", End::S), "b ˈ ɑ k s ᵻ z");
        assert!(forms("opened").contains(&("open".into(), End::Ed)));
        assert!(forms("stopped").contains(&("stop".into(), End::Ed)));
        assert!(forms("glitchin").contains(&("glitch".into(), End::In)));
        assert_eq!(forms("comin")[0], ("coming".into(), End::InFromIng));
        assert_eq!(ending("k ˈ ʌ m ɪ ŋ", End::InFromIng), "k ˈ ʌ m ɪ n");
        assert!(forms("that's").contains(&("that".into(), End::S)));
        assert_eq!(spell("zorin").as_deref(), Some("z ˈ ɑ ɹ ɪ n"));
        assert_eq!(spell("stremio").as_deref(), Some("s t ɹ ˈ ɛ m i O"));
        assert_eq!(spell("gog").as_deref(), Some("ɡ ˈ ɑ ɡ"));
        let mut words = Vec::new();
        unknown("Zorin", &lex(), &mut words);
        unknown("GPU", &lex(), &mut words);
        unknown("the", &lex(), &mut words);
        assert_eq!(words, vec!["zorin"]);
    }

    #[test]
    fn a_dash_between_words_is_a_pause() {
        let l = Lexicon::from_words(&["tomorrow", "don't", "worry"]);
        assert_eq!(for_lexicon("tomorrow—don’t worry", &l), "tomorrow, don't worry");
    }

    #[test]
    fn markdown_and_quotes_dont_cost_a_word() {
        let l = Lexicon::from_words(&["about", "across", "shade", "the", "is", "full", "moon", "three", "kilometers"]);
        assert_eq!(for_lexicon("about **3 km** across", &l), "about three kilometers across");
        assert_eq!(for_lexicon("the \"Shade\" is", &l), "the Shade is");
        assert_eq!(for_lexicon("the full\u{2011}moon", &l), for_lexicon("the full-moon", &l));
    }

    #[test]
    fn unknown_words_are_said_not_dropped() {
        let l = lex();
        assert_eq!(for_lexicon("Your screen's cozy.", &l), "Your screens cozy.");
        assert_eq!(for_lexicon("Spotify", &l), "spot if y");
        assert_eq!(for_lexicon("YouTubeMusic", &l), "You Tube Music");
        assert_eq!(for_lexicon("GPU", &l), "g p u");
    }
}
