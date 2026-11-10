//! Whisper sometimes invents stock phrases when the audio is noise or near-silence (a known artefact
//! of its training data). This removes ONLY output that consists entirely of such a phrase, so real
//! speech is never altered.

const STOCK_PHRASES: &[&str] = &[
    "thanks for watching",
    "thank you for watching",
    "thank you so much for watching",
    "please subscribe",
    "like and subscribe",
    "subscribe to my channel",
    "see you in the next video",
    "subtitles by the amara.org community",
    "subtitles by amara.org",
    "transcribed by",
    "www.mooji.org",
    "பார்த்ததற்கு நன்றி",
    "பார்த்தமைக்கு நன்றி",
    "சப்ஸ்கிரைப் செய்யுங்கள்",
];

fn normalise(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '.' || c.is_whitespace() || ('\u{0B80}'..='\u{0BFF}').contains(&c) { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .trim()
        .to_string()
}

/// True if the whole text is a known stock hallucination (or only punctuation / music symbols).
pub fn is_hallucination(text: &str) -> bool {
    let n = normalise(text);
    if n.is_empty() {
        return true;
    }
    STOCK_PHRASES.iter().any(|p| n == *p)
}

/// Collapse a phrase that Whisper repeated many times in a row (a looping failure) down to one copy.
pub fn collapse_repeats(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    for len in 1..=6usize {
        if words.len() < len * 4 {
            continue;
        }
        let unit = &words[..len];
        let mut reps = 0;
        let mut i = 0;
        while i + len <= words.len() && words[i..i + len] == *unit {
            reps += 1;
            i += len;
        }
        if reps >= 4 && i + len > words.len().saturating_sub(len - 1) && i >= words.len() {
            return unit.join(" ");
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_only_pure_stock_phrases() {
        assert!(is_hallucination("Thanks for watching!"));
        assert!(is_hallucination("  ♪ "));
        assert!(is_hallucination("பார்த்ததற்கு நன்றி."));
        assert!(!is_hallucination("Thanks for watching my son play today"));
        assert!(!is_hallucination("Please subscribe me to the newsletter"));
        assert!(!is_hallucination("வணக்கம்"));
    }

    #[test]
    fn collapses_loops_but_keeps_normal_text() {
        assert_eq!(collapse_repeats("no no no no no no"), "no");
        assert_eq!(collapse_repeats("I said no no to him"), "I said no no to him");
        assert_eq!(collapse_repeats("go go go"), "go go go");
    }
}
