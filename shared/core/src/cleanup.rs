//! Rule-based, fully offline text cleanup.
//!
//! These rules are intentionally conservative: they never add words, never
//! rewrite names, addresses or legal wording, and only touch capitalisation,
//! spacing, punctuation, a short list of filler sounds and (for the Formal
//! style) contractions. Anything smarter (rewording, tone changes) belongs to
//! the optional local language model planned for Phase 5.

use crate::numbers::convert_spoken_numbers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Return the transcription untouched.
    Raw,
    /// Fillers removed, spacing and punctuation fixed, sentences capitalised.
    Clean,
    /// Clean + informal contractions like "gonna" written out; short paragraphs.
    Professional,
    /// Professional + contractions expanded ("don't" → "do not").
    Formal,
    /// Clean without the forced full stop at the end (chat messages).
    Casual,
    /// Professional with shorter paragraphs. Never invents a greeting or sign-off.
    Email,
}

impl Style {
    pub fn parse(s: &str) -> Style {
        match s.trim().to_ascii_lowercase().as_str() {
            "raw" => Style::Raw,
            "professional" => Style::Professional,
            "formal" => Style::Formal,
            "casual" => Style::Casual,
            "email" => Style::Email,
            _ => Style::Clean,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Style::Raw => "raw",
            Style::Clean => "clean",
            Style::Professional => "professional",
            Style::Formal => "formal",
            Style::Casual => "casual",
            Style::Email => "email",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CleanupOptions {
    pub style: Style,
    pub remove_fillers: bool,
    pub spoken_numbers: bool,
}

impl Default for CleanupOptions {
    fn default() -> Self {
        CleanupOptions { style: Style::Clean, remove_fillers: true, spoken_numbers: false }
    }
}

const FILLERS: &[&str] = &["um", "umm", "uh", "uhh", "uhm", "er", "erm", "hmm", "hmmm", "mm", "mmm"];
const ABBREVIATIONS: &[&str] = &["e.g", "i.e", "etc", "vs", "mr", "mrs", "ms", "dr", "prof", "sr", "jr"];

/// Apply the selected cleanup to `text`. Newlines are preserved; each line is cleaned independently.
pub fn cleanup(text: &str, o: &CleanupOptions) -> String {
    if o.style == Style::Raw {
        return text.to_string();
    }
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let cleaned = clean_line(line, o);
        if cleaned.is_empty() && lines.last().map(|l| l.is_empty()).unwrap_or(true) {
            continue; // collapse blank runs, drop leading blanks
        }
        lines.push(cleaned);
    }
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n")
}

fn clean_line(line: &str, o: &CleanupOptions) -> String {
    let mut s = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return s;
    }
    if o.remove_fillers {
        s = remove_fillers(&s);
    }
    if o.spoken_numbers {
        s = convert_spoken_numbers(&s);
    }
    s = fix_punctuation_spacing(&s);
    if matches!(o.style, Style::Professional | Style::Formal | Style::Email) {
        s = replace_slang(&s);
    }
    if o.style == Style::Formal {
        s = expand_contractions(&s);
    }
    s = fix_pronoun_i(&s);
    s = capitalize_sentences(&s);
    if o.style != Style::Casual {
        s = ensure_terminal_punctuation(&s);
    }
    match o.style {
        Style::Email => group_sentences(&s, 3),
        Style::Professional | Style::Formal => group_sentences(&s, 5),
        _ => s,
    }
}

/// Split a token into (leading punctuation, word core, trailing punctuation).
fn split_core(tok: &str) -> (&str, &str, &str) {
    let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '\u{2019}';
    let start = tok.char_indices().find(|(_, c)| is_word(*c)).map(|(i, _)| i).unwrap_or(tok.len());
    let end = tok
        .char_indices()
        .rev()
        .find(|(_, c)| is_word(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(start);
    (&tok[..start], &tok[start..end], &tok[end..])
}

fn remove_fillers(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for tok in s.split(' ') {
        let (_, core, tail) = split_core(tok);
        let lower = core.to_lowercase();
        if !core.is_empty() && FILLERS.contains(&lower.as_str()) {
            // Keep sentence-ending punctuation by moving it onto the previous word.
            if let Some(c) = tail.chars().find(|c| matches!(c, '.' | '!' | '?')) {
                if let Some(last) = out.last_mut() {
                    if !last.ends_with(|ch: char| matches!(ch, '.' | '!' | '?')) {
                        last.push(c);
                    }
                }
            }
            continue;
        }
        out.push(tok.to_string());
    }
    out.join(" ")
}

fn fix_punctuation_spacing(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        match c {
            ',' | ';' | '!' | '?' | ':' => {
                if c != ':' || next.map(|n| n == ' ' || n.is_alphabetic()).unwrap_or(true) {
                    while out.ends_with(' ') {
                        out.pop();
                    }
                }
                out.push(c);
                if c != ':' && next.map(|n| n.is_alphabetic()).unwrap_or(false) {
                    out.push(' ');
                }
            }
            '.' => {
                // ".NET" style tokens keep their leading space.
                let attached = next.map(|n| n.is_alphanumeric()).unwrap_or(false);
                if !attached {
                    while out.ends_with(' ') {
                        out.pop();
                    }
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn recase_like(original: &str, replacement: &str) -> String {
    if original.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        let mut cs = replacement.chars();
        match cs.next() {
            Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
            None => String::new(),
        }
    } else {
        replacement.to_string()
    }
}

fn map_tokens(s: &str, f: impl Fn(&str) -> Option<String>) -> String {
    s.split(' ')
        .map(|tok| {
            let (lead, core, tail) = split_core(tok);
            match f(core) {
                Some(rep) => format!("{}{}{}", lead, recase_like(core, &rep), tail),
                None => tok.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn replace_slang(s: &str) -> String {
    map_tokens(s, |core| {
        let r = match core.to_lowercase().as_str() {
            "gonna" => "going to",
            "wanna" => "want to",
            "gotta" => "got to",
            "kinda" => "kind of",
            "sorta" => "sort of",
            "lemme" => "let me",
            "gimme" => "give me",
            _ => return None,
        };
        Some(r.to_string())
    })
}

fn expand_contractions(s: &str) -> String {
    map_tokens(s, |core| {
        let norm = core.replace('\u{2019}', "'");
        let lower = norm.to_lowercase();
        match lower.as_str() {
            "won't" => return Some("will not".into()),
            "can't" => return Some("cannot".into()),
            "shan't" => return Some("shall not".into()),
            "i'm" => return Some("I am".into()),
            _ => {}
        }
        if let Some(base) = lower.strip_suffix("n't") {
            if !base.is_empty() {
                return Some(format!("{} not", base));
            }
        }
        for (suffix, word) in [("'re", "are"), ("'ll", "will"), ("'ve", "have")] {
            if let Some(base) = lower.strip_suffix(suffix) {
                if !base.is_empty() {
                    return Some(format!("{} {}", base, word));
                }
            }
        }
        None
    })
}

fn fix_pronoun_i(s: &str) -> String {
    s.split(' ')
        .map(|tok| {
            let (lead, core, tail) = split_core(tok);
            let norm = core.replace('\u{2019}', "'").to_lowercase();
            if matches!(norm.as_str(), "i" | "i'm" | "i'll" | "i've" | "i'd") && core.is_ascii() {
                let mut fixed = String::from("I");
                fixed.push_str(&core[1..]);
                format!("{}{}{}", lead, fixed, tail)
            } else {
                tok.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ends_with_abbreviation(prefix: &str) -> bool {
    let word = prefix
        .rsplit(' ')
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_lowercase();
    ABBREVIATIONS.contains(&word.as_str())
}

fn capitalize_sentences(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap_next = true;
    let mut prev_terminal = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if prev_terminal && !ends_with_abbreviation(&out) {
                cap_next = true;
            }
            prev_terminal = false;
            out.push(c);
            continue;
        }
        if cap_next && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            cap_next = false;
        } else {
            if cap_next && c.is_alphanumeric() {
                cap_next = false;
            }
            out.push(c);
        }
        prev_terminal = matches!(c, '.' | '!' | '?');
    }
    out
}

fn ensure_terminal_punctuation(s: &str) -> String {
    let mut t = s.trim_end().to_string();
    match t.chars().last() {
        None => t,
        Some(',') | Some(';') | Some(':') => {
            t.pop();
            t.push('.');
            t
        }
        Some(c) if matches!(c, '.' | '!' | '?' | '\u{2026}' | '"' | '\'' | ')' | ']' | '}' | '\u{201D}' | '\u{2019}') => t,
        Some(_) => {
            t.push('.');
            t
        }
    }
}

fn split_sentences(s: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut prev_terminal = false;
    for c in s.chars() {
        if c == ' ' && prev_terminal && !ends_with_abbreviation(&current) {
            sentences.push(std::mem::take(&mut current));
            prev_terminal = false;
            continue;
        }
        current.push(c);
        prev_terminal = matches!(c, '.' | '!' | '?');
    }
    if !current.trim().is_empty() {
        sentences.push(current);
    }
    sentences
}

fn group_sentences(s: &str, per_paragraph: usize) -> String {
    let sentences = split_sentences(s);
    if sentences.len() <= per_paragraph {
        return s.to_string();
    }
    sentences
        .chunks(per_paragraph)
        .map(|c| c.join(" "))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, style: Style) -> String {
        cleanup(text, &CleanupOptions { style, remove_fillers: true, spoken_numbers: false })
    }

    #[test]
    fn raw_is_untouched() {
        let t = "  um hello   world ";
        assert_eq!(run(t, Style::Raw), t);
    }

    #[test]
    fn clean_basic() {
        assert_eq!(run("um so i think , uh we should go", Style::Clean), "So I think, we should go.");
        assert_eq!(run("hello world", Style::Clean), "Hello world.");
        assert_eq!(run("what time is it ? i'm late", Style::Clean), "What time is it? I'm late.");
    }

    #[test]
    fn filler_before_period_keeps_period() {
        assert_eq!(run("we are done um. next item", Style::Clean), "We are done. Next item.");
    }

    #[test]
    fn casual_has_no_forced_period() {
        assert_eq!(run("ok see you at five", Style::Casual), "Ok see you at five");
    }

    #[test]
    fn formal_expands_contractions_but_not_names() {
        assert_eq!(
            run("i don't think we can't proceed and it's fine", Style::Formal),
            "I do not think we cannot proceed and it's fine."
        );
        assert_eq!(run("Don't tell Raman Nair", Style::Formal), "Do not tell Raman Nair.");
    }

    #[test]
    fn professional_expands_slang() {
        assert_eq!(run("we're gonna ship it", Style::Professional), "We're going to ship it.");
    }

    #[test]
    fn abbreviations_do_not_trigger_capitals() {
        assert_eq!(run("bring pens e.g. blue ones", Style::Clean), "Bring pens e.g. blue ones.");
    }

    #[test]
    fn decimals_and_dotted_names_survive() {
        assert_eq!(run("pi is 3.14 and I like .NET", Style::Clean), "Pi is 3.14 and I like .NET.");
    }

    #[test]
    fn tamil_text_is_not_mangled() {
        assert_eq!(run("வணக்கம் நண்பா", Style::Clean), "வணக்கம் நண்பா.");
    }

    #[test]
    fn email_groups_paragraphs() {
        let t = "one. two. three. four.";
        assert_eq!(run(t, Style::Email), "One. Two. Three.\n\nFour.");
    }

    #[test]
    fn newlines_preserved_and_blank_runs_collapsed() {
        assert_eq!(run("first line\n\n\nsecond line", Style::Clean), "First line.\n\nSecond line.");
    }

    #[test]
    fn numbers_optional() {
        let o = CleanupOptions { style: Style::Clean, remove_fillers: true, spoken_numbers: true };
        assert_eq!(cleanup("i paid twenty five rupees", &o), "I paid 25 rupees.");
    }
}
