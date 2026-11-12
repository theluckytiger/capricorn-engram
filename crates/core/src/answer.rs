//! Whether something said or typed is a yes or a no to a question of hers ("Do this?", "Did you mean Stremio?"). In one
//! place: her ears need it to know an answer when they hear one, and her brain to act on it.

/// Words that don't change the answer.
const LITTLE: &[&str] = &["capri", "capricorn", "copy", "um", "uh", "oh", "hey", "okay", "ok", "okey", "please", "thanks", "thank", "you"];

/// `Some(true)` for a yes, `Some(false)` for a no, `None` for anything else (a new request).
pub fn yes_no(text: &str) -> Option<bool> {
    let s: String = text.to_lowercase().replace('’', "'").chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '\'').collect();
    let raw = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if matches!(raw.as_str(), "uh huh" | "mhm" | "mm hmm" | "mmhmm" | "mm hm") {
        return Some(true);
    }
    let mut words: Vec<&str> = raw.split(' ').filter(|w| !w.is_empty() && !LITTLE.contains(w)).collect();
    // "Yes, yes." "Yeah yeah yeah": once is enough.
    words.dedup();
    // "Okay." by itself is a yes; with more, it's only how the sentence starts ("okay, no").
    if words.is_empty() {
        return raw.split(' ').any(|w| matches!(w, "okay" | "ok" | "okey")).then_some(true);
    }
    let s = words.join(" ");
    match s.as_str() {
        "yes" | "yeah" | "yep" | "yup" | "ya" | "yea" | "sure" | "correct" | "right" | "that one" | "do it" | "do that" | "go" | "go on" | "go ahead" | "go for it" | "open it" | "yes open it" | "yes that one" | "affirmative" | "exactly" | "yes i did" | "i did" | "alright" | "all right" | "fine" | "sure thing" | "sounds good" | "absolutely" | "definitely" | "of course" | "proceed" | "approve" | "approved" | "allow it" | "let's go" | "lets go" => return Some(true),
        "no" | "nope" | "nah" | "not that" | "wrong" | "neither" | "no i didn't" | "not that one" | "no way" | "don't" | "dont" | "don't do it" | "dont do it" | "don't do that" | "stop" | "cancel" | "never mind" | "nevermind" | "not now" | "later" | "leave it" | "skip it" | "deny" | "denied" => return Some(false),
        _ => {}
    }
    // Several of them together ("yes, do it", "yeah, that one, go ahead", "no no, wrong one").
    const YES: &[&str] = &["yes", "yeah", "yep", "yup", "ya", "yea", "sure", "correct", "right", "exactly", "affirmative", "alright", "fine", "absolutely", "definitely"];
    const YES_TOO: &[&str] = &["do", "it", "go", "ahead", "open", "that", "one", "that's", "is", "the", "i", "did", "mean", "meant", "of", "course", "all", "for"];
    const NO: &[&str] = &["no", "nope", "nah", "don't", "dont"];
    const NO_TOO: &[&str] = &["not", "that", "one", "wrong", "neither", "i", "didn't", "the", "mean", "it", "never", "mind", "do", "now", "way", "stop", "cancel"];
    if words.iter().any(|w| YES.contains(w)) && words.iter().all(|w| YES.contains(w) || YES_TOO.contains(w)) {
        return Some(true);
    }
    if words.iter().any(|w| NO.contains(w)) && words.iter().all(|w| NO.contains(w) || NO_TOO.contains(w)) {
        return Some(false);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::yes_no;

    #[test]
    fn yes_however_its_said() {
        for t in ["Yes.", "Yeah, please.", "Yes, yes.", "Yeah yeah yeah", "Yes, do it.", "Yeah, that's the one.", "Okay.", "OK", "Okay, do it.", "Alright.", "All right.", "Do it.", "Go ahead.", "Go for it", "Sure.", "Capri, yes.", "Fine.", "Mhm.", "Uh huh.", "Yep, go ahead"] {
            assert_eq!(yes_no(t), Some(true), "{t}");
        }
    }

    #[test]
    fn no_however_its_said() {
        for t in ["No.", "Nope.", "No, no, not that one.", "Don't.", "Don’t do it.", "Stop.", "Cancel.", "Never mind.", "Not now.", "Okay, no.", "No way."] {
            assert_eq!(yes_no(t), Some(false), "{t}");
        }
    }

    #[test]
    fn anything_else_is_a_new_request() {
        for t in ["Open Steam.", "Yes, and then play the first video.", "Do I still have Cyberpunk?", "Thanks.", "Uh.", "", "Go to my other screen.", "Do the dishes."] {
            assert_eq!(yes_no(t), None, "{t}");
        }
    }
}
