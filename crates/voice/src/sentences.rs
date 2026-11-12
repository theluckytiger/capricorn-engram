//! Splitting a line into sentences, so she starts talking after the first one is made instead of the whole line.

/// Sentences, with their ending punctuation. Abbreviations and decimals don't end one ("Dr. Who", "3.5 mm").
pub fn split(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        if !matches!(c, '.' | '!' | '?' | '…') {
            continue;
        }
        let next = chars.get(i + 1).copied();
        let prev = if i > 0 { chars.get(i - 1).copied() } else { None };
        // 3.5 — a point between digits.
        if c == '.' && prev.is_some_and(|p| p.is_ascii_digit()) && next.is_some_and(|n| n.is_ascii_digit()) {
            continue;
        }
        // "..." and "?!" end together.
        if next.is_some_and(|n| matches!(n, '.' | '!' | '?' | '"' | '”' | '\'' | ')')) {
            continue;
        }
        if next.is_some_and(|n| !n.is_whitespace()) {
            continue;
        }
        // "Wait... what?" — the next word in lowercase carries on the same sentence.
        if chars[i + 1..].iter().find(|c| !c.is_whitespace()).is_some_and(|c| c.is_lowercase()) {
            continue;
        }
        let word = cur.split_whitespace().last().unwrap_or("").to_lowercase();
        if matches!(word.as_str(), "mr." | "mrs." | "ms." | "dr." | "st." | "vs." | "e.g." | "i.e." | "etc.") {
            continue;
        }
        let s = cur.trim().to_string();
        if !s.is_empty() {
            out.push(s);
        }
        cur.clear();
    }
    let s = cur.trim().to_string();
    if !s.is_empty() {
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn sentences() {
        assert_eq!(split("Paused. Try not to miss me."), vec!["Paused.", "Try not to miss me."]);
        assert_eq!(split("It's 3.5 mm today! Wild, right?"), vec!["It's 3.5 mm today!", "Wild, right?"]);
        assert_eq!(split("Wait... what?! No."), vec!["Wait... what?!", "No."]);
        assert_eq!(split("no punctuation at all"), vec!["no punctuation at all"]);
    }
}
