//! Her names in what was heard: "Capri" and "Capricorn", at the start of an utterance (after a "hey" or two), the
//! way speech recognition actually writes them ("Kapri", "Caprey", "Copri", "Capricorns"…), and not real words
//! that merely start the same way ("caprese", "capricious", "copper").

/// What follows her name, if the utterance starts with it. `Some("")` is her name alone.
pub fn find(text: &str) -> Option<String> {
    let words: Vec<String> = text.split_whitespace().map(norm).filter(|w| !w.is_empty()).collect();
    // "hey", "okay so", "um"… before her name.
    let mut i = 0;
    while i < words.len() && i < 3 && FILLER.contains(&words[i].as_str()) {
        i += 1;
    }
    let w = words.get(i)?;
    // Keep the original words (case, punctuation) for the request.
    let original: Vec<&str> = text.split_whitespace().collect();
    // Parakeet often writes "Capri" as "Copy". Said to her, it's alone ("Copy.") or set off by a pause the
    // recognizer marks ("Copy, minimize all my windows"); "Copy that" and "copy the file" are just copy.
    let addressed = original.get(i).is_some_and(|o| o.ends_with([',', '.', '!', '?'])) && !words.get(i + 1).is_some_and(|n| COPY_OBJECTS.contains(&n.as_str()));
    let copy_is_her = w == "copy" && (words.len() == i + 1 || addressed);
    // How Parakeet has written "Capri" said alone ("Captain.", "Capital."): her name only when that's all there is.
    // "Cabry." alone too: her name with a b (not in a sentence, where "cobra" and "Kubrick" would be her).
    let soft = w.len() >= 4 && w.as_bytes()[2] == b'b' && w.is_char_boundary(3) && is_name(&format!("{}p{}", &w[..2], &w[3..]));
    let alone = words.len() == i + 1 && (ALONE.contains(&w.as_str()) || soft);
    if !is_name(w) && !copy_is_her && !alone {
        return None;
    }
    // Her name said twice, or split ("Capri Capri Korn"): all of it is her name, not the request.
    let mut j = i + 1;
    while let Some(n) = words.get(j)
        && (is_name(n) || NAME_TAIL.contains(&n.as_str()))
    {
        j += 1;
    }
    let rest = original.get(j..).map(|r| r.join(" ")).unwrap_or_default();
    Some(rest.trim_start_matches([',', '.', '!', '?', ' ']).trim().to_string())
}

/// Her name at the end, set off by a comma ("Press enter, Capri."): what came before it.
pub fn trailing(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches(['.', '!', '?']);
    let (before, last) = t.rsplit_once(',')?;
    let last = last.trim();
    if last.split_whitespace().count() != 1 || !is_name(&norm(last)) {
        return None;
    }
    let before = before.trim();
    (!before.is_empty()).then(|| before.to_string())
}

/// "Capri", said alone, as Parakeet has written it (2026-11-03 journal).
const ALONE: &[&str] = &["captain", "capital"];

/// The end of "Capricorn", heard as its own word after "Capri".
const NAME_TAIL: &[&str] = &["corn", "korn", "con", "cone", "coin", "born", "horn"];

/// Her name anywhere in what was heard, for sentences heard while she talks (her own words come first in those):
/// what follows its last mention, `Some("")` if nothing does. Words in `hers` (name-like words in her own line)
/// don't count, and "copy" counts only when a pause sets it off ("…copy, stop").
pub fn after_name(text: &str, hers: &[String]) -> Option<String> {
    let original: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<String> = original.iter().map(|w| norm(w)).collect();
    let mut at = None;
    for (i, w) in words.iter().enumerate() {
        if w.is_empty() || hers.contains(w) {
            continue;
        }
        let set_off = original[i].ends_with([',', '.', '!', '?']);
        let copy = w == "copy" && set_off && !words.get(i + 1).is_some_and(|n| COPY_OBJECTS.contains(&n.as_str()));
        if is_name(w) || copy {
            at = Some(i);
        }
    }
    let rest = original[at? + 1..].join(" ");
    Some(rest.trim_start_matches([',', '.', '!', '?', ' ']).trim().to_string())
}

/// The name-like words in a line of hers: when she says them, the mic hearing them isn't someone calling her.
pub fn names_in(line: &str) -> Vec<String> {
    line.split_whitespace().map(norm).filter(|w| w == "copy" || is_name(w)).collect()
}

/// How much of what was heard is words from her line (0..1): her own voice, back through the mic.
pub fn echo_share(heard: &str, line: &str) -> f32 {
    let said: std::collections::HashSet<String> = line.split_whitespace().map(norm).collect();
    let words: Vec<String> = heard.split_whitespace().map(norm).filter(|w| w.chars().count() >= 2).collect();
    if words.is_empty() {
        return 0.0;
    }
    words.iter().filter(|w| said.contains(*w)).count() as f32 / words.len() as f32
}

/// A request that stopped mid-thought ("select the text, remove it and type"): Parakeet ends a finished sentence
/// with a stop, and a sentence doesn't end on "and" or "the".
pub fn unfinished(text: &str) -> bool {
    let t = text.trim();
    let last = t.split_whitespace().last().map(norm).unwrap_or_default();
    !t.ends_with(['.', '?', '!']) || DANGLING_END.contains(&last.as_str())
}

/// No Cyrillic in it. Said with an accent, a whole request can come out in Cyrillic letters ("Капри
/// Майхоум из…"): her name still counts, the rest has to be said again.
pub fn english(text: &str) -> bool {
    !text.chars().any(|c| matches!(c, '\u{0400}'..='\u{04FF}'))
}

/// A word as compared: no punctuation around it, lowercase, Cyrillic letters as their Latin sounds ("Капри" is
/// "kapri").
fn norm(w: &str) -> String {
    let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase();
    if english(&w) {
        return w;
    }
    w.chars()
        .map(|c| match c {
            'а' | 'ъ' => "a",
            'б' => "b",
            'в' => "v",
            'г' => "g",
            'д' => "d",
            'е' | 'э' => "e",
            'ж' => "zh",
            'з' => "z",
            'и' => "i",
            'й' | 'ы' => "y",
            'к' => "k",
            'л' => "l",
            'м' => "m",
            'н' => "n",
            'о' => "o",
            'п' => "p",
            'р' => "r",
            'с' => "s",
            'т' => "t",
            'у' => "u",
            'ф' => "f",
            'х' => "h",
            'ц' => "ts",
            'ч' => "ch",
            'ш' => "sh",
            'щ' => "sht",
            'ю' => "yu",
            'я' => "ya",
            'ё' => "yo",
            c if c.is_ascii_alphanumeric() || c == '\'' => return c.to_string(),
            _ => "",
        }
        .to_string())
        .collect()
}

/// A request as meant, where Parakeet is known to mishear it (all heard in real use): "Unmute my Discord Mike" →
/// "…mic", "close the tub" → "tab", "On mute." → "Unmute.", "I opened the tub I just closed" → "Reopen the tab…".
/// Only these, only in those places; everything else stays as heard.
pub fn fix(text: &str) -> String {
    let mut words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    let bare = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    // Swaps a word for another, keeping the punctuation around it.
    let swap = |w: &str, to: &str| {
        let start = w.find(|c: char| c.is_alphanumeric()).unwrap_or(0);
        let end = w.rfind(|c: char| c.is_alphanumeric()).map_or(w.len(), |i| i + w[i..].chars().next().map_or(1, char::len_utf8));
        format!("{}{to}{}", &w[..start], &w[end..])
    };
    for i in 0..words.len() {
        let before = if i > 0 { bare(&words[i - 1]) } else { String::new() };
        match bare(&words[i]).as_str() {
            "mike" if matches!(before.as_str(), "my" | "the" | "discord" | "your") => words[i] = swap(&words[i], "mic"),
            // "Press on Call of Duty in Steven": Steam (it searched YouTube for Steven).
            "steven" if matches!(before.as_str(), "in" | "on" | "open" | "from") => words[i] = swap(&words[i], "Steam"),
            "tub" | "tubs" if matches!(before.as_str(), "the" | "this" | "that" | "a" | "new" | "next" | "last" | "previous" | "other" | "every") => {
                words[i] = swap(&words[i], if words[i].to_lowercase().contains("tubs") { "tabs" } else { "tab" })
            }
            _ => {}
        }
    }
    let lower: Vec<String> = words.iter().map(|w| bare(w)).collect();
    // "On mute." / "Un mute my mic": unmute.
    if lower.len() >= 2 && matches!(lower[0].as_str(), "on" | "un") && lower[1] == "mute" {
        let rest = words[1].trim_start_matches(|c: char| c.is_alphanumeric()).to_string();
        words.splice(0..2, [format!("Unmute{rest}")]);
    } else if lower.len() >= 4 && lower[..3] == ["i", "opened", "the"] && matches!(lower[3].as_str(), "tab" | "tub") {
        words.splice(0..2, ["Reopen".to_string()]);
    }
    words.join(" ")
}

/// Whether a sentence heard while she's listening is a request, not a stray sound: two real words or more, or one
/// of the one-word commands. "Uh.", "T.", "Okay, my", "Ne?" aren't.
pub fn meaningful(text: &str) -> bool {
    let mut words: Vec<String> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| w.chars().count() >= 2 && !FILLER.contains(&w.as_str()))
        .collect();
    // "Okay, my" trails off; "How are you?" is asked. (Dropping these words anywhere threw "How are you?" away.)
    if !text.trim_end().ends_with(['?', '!']) {
        while words.last().is_some_and(|w| DANGLING.contains(&w.as_str())) {
            words.pop();
        }
    }
    match words.len() {
        0 => false,
        1 => ONE_WORD.contains(&words[0].as_str()),
        _ => true,
    }
}

const ONE_WORD: &[&str] = &["pause", "play", "stop", "next", "skip", "previous", "back", "mute", "unmute", "louder", "quieter", "yes", "no", "yeah", "nope", "cancel", "thanks", "weather", "time", "minimize", "restore", "screenshot"];

/// Words that only start a sentence ("okay, my…").
const DANGLING: &[&str] = &["my", "the", "can", "could", "would", "will", "what", "is", "are", "you", "i", "it", "to", "please", "just", "maybe"];

/// After "copy." these mean the word copy, not her ("Copy. That's it.", "Copy, paste").
const COPY_OBJECTS: &[&str] = &["that", "this", "it", "paste", "the", "and", "all", "me", "you", "over", "roger"];

/// Words a finished request doesn't end on.
const DANGLING_END: &[&str] = &["and", "or", "but", "then", "the", "a", "an", "to", "type", "write", "say", "of", "with", "into", "called", "named", "my", "your", "for", "um", "uh"];

const FILLER: &[&str] = &["hey", "hay", "a", "eh", "hi", "hello", "okay", "ok", "yo", "so", "um", "uh", "oh", "ah", "and", "alright", "right", "well", "listen", "hm", "hmm"];

/// Real words that start like her name.
const NOT_HER: &[&str] = &["caprese", "capricious", "capriciously", "capriccio", "caprine", "capris", "copper", "coppery", "capri-pants", "caper", "capers", "capercaillie"];

fn is_name(w: &str) -> bool {
    if NOT_HER.contains(&w) {
        return false;
    }
    let w = w.strip_suffix("'s").unwrap_or(w);
    // c/k, then a/o/u, then p (or pp), then r: capri, kapri, copri, cuppri, capreee, capricorn…
    let b = w.as_bytes();
    if b.len() < 4 {
        return false;
    }
    let mut j = 0;
    if !matches!(b[j], b'c' | b'k') {
        return false;
    }
    j += 1;
    if !matches!(b[j], b'a' | b'o' | b'u') {
        return false;
    }
    j += 1;
    if b[j] != b'p' {
        return false;
    }
    j += 1;
    if b.get(j) == Some(&b'p') {
        j += 1;
    }
    if b.get(j) != Some(&b'r') {
        return false;
    }
    // Something vowel-ish after the r ("capr" alone is a cut-off, still her), and not too long unless it's
    // "capricorn" and its plurals.
    w.len() <= 7 || ["capricorn", "kapricorn", "caprikorn", "kaprikorn"].iter().any(|n| w.starts_with(n))
}

#[cfg(test)]
mod tests {
    use super::find;

    #[test]
    fn names_as_written() {
        for s in ["Capri.", "Kapri", "Caprey.", "Copri, hi", "A copry", "Hey Capri.", "okay so capri", "Capricorn", "Capricorns?", "Copy."] {
            assert!(find(s).is_some(), "{s}");
        }
    }

    #[test]
    fn request_after_the_name() {
        assert_eq!(find("Capri, mute my mic.").as_deref(), Some("mute my mic."));
        assert_eq!(find("Hey Capri what's the weather").as_deref(), Some("what's the weather"));
        assert_eq!(find("Capri.").as_deref(), Some(""));
        // how Parakeet wrote "Capri, minimize all my windows and then open Steam."
        assert_eq!(find("Copy, minimize all my windows and then open Steam.").as_deref(), Some("minimize all my windows and then open Steam."));
    }

    #[test]
    fn steam_misheard() {
        assert_eq!(super::fix("Press on Call of Duty in Steven."), "Press on Call of Duty in Steam.");
        assert_eq!(super::fix("Steven Spielberg."), "Steven Spielberg.");
    }

    #[test]
    fn her_name_at_the_end() {
        use super::trailing;
        assert_eq!(trailing("Press enter, Capri.").as_deref(), Some("Press enter"));
        assert_eq!(trailing("Open Steam, Capricorn!").as_deref(), Some("Open Steam"));
        assert_eq!(trailing("I was at Capri last summer."), None);
        assert_eq!(trailing("Well, hello."), None);
    }

    #[test]
    fn misheard_alone_is_her_name() {
        assert_eq!(find("Cabry.").as_deref(), Some(""));
        assert_eq!(find("Cobra Kai is on."), None);
        assert_eq!(find("Captain.").as_deref(), Some(""));
        assert_eq!(find("Capital.").as_deref(), Some(""));
        assert_eq!(find("Captain America is on tonight."), None);
    }

    #[test]
    fn her_name_twice_is_just_her_name() {
        assert_eq!(find("Capri Capri Korn").as_deref(), Some(""));
        assert_eq!(find("Capri Korn, open Steam.").as_deref(), Some("open Steam."));
    }

    #[test]
    fn stray_sounds_arent_requests() {
        use super::meaningful;
        for s in ["Uh.", "T.", "Okay, my", "Ne?", "um um"] {
            assert!(!meaningful(s), "{s}");
        }
        for s in ["pause", "Yes.", "open discord", "what's the weather", "Mute my mic.", "How are you?", "Is it raining", "Are you there?"] {
            assert!(meaningful(s), "{s}");
        }
    }

    #[test]
    fn in_cyrillic() {
        // How Parakeet wrote "Capri" said with an accent, three times in one evening.
        assert_eq!(find("Капри").as_deref(), Some(""));
        assert_eq!(find("Капри.").as_deref(), Some(""));
        assert!(find("Каприкорн").is_some());
        assert!(!super::english("Майхоум из Кузаровец"));
        assert!(find("Добре, тук може да имаме попадение").is_none());
    }

    #[test]
    fn over_her_voice() {
        use super::{after_name, names_in};
        // What the mic hears when she's mid-line and you call her: her words, then yours.
        assert_eq!(after_name("Rain's starting at four tomorrow Capri, stop.", &[]).as_deref(), Some("stop."));
        assert_eq!(after_name("stays wet all day. Capri.", &[]).as_deref(), Some(""));
        assert_eq!(after_name("all day copy, pause the music", &[]).as_deref(), Some("pause the music"));
        assert!(after_name("I'll copy that for you", &[]).is_none(), "copy, not her");
        assert!(after_name("Rain's starting at four tomorrow", &[]).is_none());
        // Her own line says "Capricorn": hearing it is her, not you.
        let hers = names_in("I'm Capricorn's ghost, choom.");
        assert!(after_name("I'm Capricorn's ghost", &hers).is_none());
        assert_eq!(after_name("I'm Capricorn's ghost Capri, stop", &hers).as_deref(), Some("stop"));
    }

    #[test]
    fn her_own_voice() {
        use super::echo_share;
        let line = "Rain's starting at 04:00 tomorrow, stays wet all day.";
        assert!(echo_share("starting at tomorrow, stays wet all day", line) > 0.8);
        assert!(echo_share("pause the music", line) < 0.2);
    }

    #[test]
    fn mid_thought() {
        use super::unfinished;
        for s in ["Select the text, remove it and type", "open steam and", "type the.", "Play Call of Duty on Steam"] {
            assert!(unfinished(s), "{s}");
        }
        for s in ["Select the text, remove it, and type Google.", "Pause.", "What's the weather like?", "Mute me."] {
            assert!(!unfinished(s), "{s}");
        }
    }

    #[test]
    fn misheard_the_known_way() {
        use super::fix;
        assert_eq!(fix("Unmute my Discord Mike."), "Unmute my Discord mic.");
        assert_eq!(fix("on mute."), "Unmute.");
        assert_eq!(fix("On mute my mic"), "Unmute my mic");
        assert_eq!(fix("I opened the tub I just closed."), "Reopen the tab I just closed.");
        assert_eq!(fix("close this tub"), "close this tab");
        // Not those words in those places: untouched.
        assert_eq!(fix("tell Mike I'm on in five"), "tell Mike I'm on in five");
        assert_eq!(fix("I'm on mute right now"), "I'm on mute right now");
        assert_eq!(fix("fill the bath tub"), "fill the bath tub");
    }

    #[test]
    fn not_her() {
        for s in ["Copy that.", "Copy that. See you tomorrow.", "copy the file to my desktop", "Copy, paste, done.", "I had a caprese salad","capricious weather", "copper wire", "I love Capri pants", "the capers", "so I was in Capri last summer"] {
            // "in Capri" isn't at the start; a lone "Capri" sentence about the island is a risk we accept.
            assert!(find(s).is_none(), "{s}");
        }
    }
}
