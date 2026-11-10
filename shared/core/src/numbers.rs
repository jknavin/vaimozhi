//! Conservative spoken-number conversion ("twenty five" → "25").
//!
//! Single number words ("one", "ten") are left alone because they are usually
//! ordinary words ("one of them"). Only runs of two or more number words, or
//! runs containing a scale word ("hundred", "thousand", "lakh", "crore",
//! "million"), are converted. An invalid continuation ends the run instead of
//! guessing.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Unit,
    Teen,
    Tens,
    Hundred,
    Scale,
    And,
}

fn word_value(w: &str) -> Option<(Kind, u64)> {
    Some(match w {
        "zero" => (Kind::Unit, 0),
        "one" => (Kind::Unit, 1),
        "two" => (Kind::Unit, 2),
        "three" => (Kind::Unit, 3),
        "four" => (Kind::Unit, 4),
        "five" => (Kind::Unit, 5),
        "six" => (Kind::Unit, 6),
        "seven" => (Kind::Unit, 7),
        "eight" => (Kind::Unit, 8),
        "nine" => (Kind::Unit, 9),
        "ten" => (Kind::Teen, 10),
        "eleven" => (Kind::Teen, 11),
        "twelve" => (Kind::Teen, 12),
        "thirteen" => (Kind::Teen, 13),
        "fourteen" => (Kind::Teen, 14),
        "fifteen" => (Kind::Teen, 15),
        "sixteen" => (Kind::Teen, 16),
        "seventeen" => (Kind::Teen, 17),
        "eighteen" => (Kind::Teen, 18),
        "nineteen" => (Kind::Teen, 19),
        "twenty" => (Kind::Tens, 20),
        "thirty" => (Kind::Tens, 30),
        "forty" => (Kind::Tens, 40),
        "fifty" => (Kind::Tens, 50),
        "sixty" => (Kind::Tens, 60),
        "seventy" => (Kind::Tens, 70),
        "eighty" => (Kind::Tens, 80),
        "ninety" => (Kind::Tens, 90),
        "hundred" => (Kind::Hundred, 100),
        "thousand" => (Kind::Scale, 1_000),
        "lakh" | "lakhs" => (Kind::Scale, 100_000),
        "million" => (Kind::Scale, 1_000_000),
        "crore" | "crores" => (Kind::Scale, 10_000_000),
        "and" => (Kind::And, 0),
        _ => return None,
    })
}

/// Split a token into (lowercase alphabetic core, trailing punctuation).
fn split_token(tok: &str) -> Option<(String, &str)> {
    let end = tok
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphabetic())
        .map(|(i, _)| i)
        .unwrap_or(tok.len());
    if end == 0 {
        return None;
    }
    let (core, tail) = tok.split_at(end);
    // Anything after the word must be punctuation, not more word characters.
    if tail.chars().any(|c| c.is_alphanumeric()) {
        return None;
    }
    Some((core.to_ascii_lowercase(), tail))
}

/// Parse the longest valid number starting at `toks[0]`.
/// Returns (value, tokens consumed, trailing punctuation of the last token).
fn parse_run<'a>(toks: &[&'a str]) -> Option<(u64, usize, &'a str)> {
    let mut total: u64 = 0;
    let mut chunk: u64 = 0;
    let mut prev: Option<Kind> = None;
    let mut consumed = 0usize; // tokens consumed including a pending-free "and"
    let mut number_words = 0usize;
    let mut has_big = false;
    let mut best: Option<(u64, usize, &'a str, usize, bool)> = None;

    let mut j = 0;
    while j < toks.len() {
        let Some((core, tail)) = split_token(toks[j]) else { break };
        let Some((kind, val)) = word_value(&core) else { break };

        let ok = match kind {
            Kind::And => {
                // Only between a hundred/scale and a following number word, with no punctuation.
                let next_is_num = toks
                    .get(j + 1)
                    .and_then(|t| split_token(t))
                    .and_then(|(c, _)| word_value(&c))
                    .map(|(k, _)| k != Kind::And && k != Kind::Hundred && k != Kind::Scale)
                    .unwrap_or(false);
                tail.is_empty() && matches!(prev, Some(Kind::Hundred) | Some(Kind::Scale)) && next_is_num
            }
            Kind::Unit => match prev {
                Some(Kind::Unit) | Some(Kind::Teen) => false,
                Some(Kind::Tens) => val >= 1 && chunk % 10 == 0,
                _ => true,
            },
            Kind::Teen => !matches!(prev, Some(Kind::Unit) | Some(Kind::Teen) | Some(Kind::Tens)),
            Kind::Tens => !matches!(prev, Some(Kind::Unit) | Some(Kind::Teen) | Some(Kind::Tens)),
            Kind::Hundred => matches!(prev, Some(Kind::Unit)) && (1..=9).contains(&chunk),
            Kind::Scale => chunk > 0 && !matches!(prev, Some(Kind::Scale)),
        };
        if !ok {
            break;
        }

        match kind {
            Kind::Unit | Kind::Teen | Kind::Tens => {
                chunk += val;
                number_words += 1;
            }
            Kind::Hundred => {
                chunk *= 100;
                has_big = true;
                number_words += 1;
            }
            Kind::Scale => {
                total += chunk * val;
                chunk = 0;
                has_big = true;
                number_words += 1;
            }
            Kind::And => {}
        }
        prev = Some(kind);
        j += 1;
        consumed = j;

        if kind != Kind::And && (number_words >= 2 || has_big) {
            best = Some((total + chunk, consumed, tail, number_words, has_big));
        }
        if !tail.is_empty() {
            break; // punctuation ends the run
        }
    }
    let _ = consumed;
    best.map(|(v, c, t, _, _)| (v, c, t))
}

/// Convert runs of spoken English numbers to digits.
pub fn convert_spoken_numbers(text: &str) -> String {
    let toks: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if let Some((value, consumed, tail)) = parse_run(&toks[i..]) {
            out.push(format!("{}{}", value, tail));
            i += consumed;
        } else {
            out.push(toks[i].to_string());
            i += 1;
        }
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::convert_spoken_numbers as c;

    #[test]
    fn converts_compound_numbers() {
        assert_eq!(c("twenty five"), "25");
        assert_eq!(c("one hundred and fifty rupees"), "150 rupees");
        assert_eq!(c("two thousand twenty six"), "2026");
        assert_eq!(c("five lakh"), "500000");
        assert_eq!(c("I paid three hundred, then left"), "I paid 300, then left");
        assert_eq!(c("one thousand and one"), "1001");
    }

    #[test]
    fn leaves_ordinary_words_alone() {
        assert_eq!(c("one of them"), "one of them");
        assert_eq!(c("ten"), "ten");
        assert_eq!(c("hundred"), "hundred");
        assert_eq!(c("one two"), "one two");
        assert_eq!(c("back and forth"), "back and forth");
    }

    #[test]
    fn stops_at_invalid_continuation() {
        assert_eq!(c("twenty five six"), "25 six");
    }
}
