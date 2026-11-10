//! VAIMOZHI shared core: language modes,
//! rule-based text cleanup, spoken-number conversion and the Whisper model catalog.
//!
//! Everything here is pure Rust with no dependencies so the same logic can be
//! used by the Windows app now and by the Android app (via JNI) later.

pub mod cleanup;
pub mod hallucination;
pub mod models;
pub mod numbers;

pub use cleanup::{cleanup, CleanupOptions, Style};

/// Recognition mode chosen by the user. Only Tamil and English are supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// Tamil speech, Tamil script output.
    Tamil,
    /// English speech, English output.
    English,
    /// Tamil or English, decided automatically for each stretch of speech.
    Auto,
}

impl Language {
    /// Unknown or retired values (for example an old "tanglish" or "mixed" setting) become Auto.
    pub fn parse(s: &str) -> Language {
        match s.trim().to_ascii_lowercase().as_str() {
            "ta" | "tamil" => Language::Tamil,
            "en" | "english" => Language::English,
            _ => Language::Auto,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Language::Tamil => "tamil",
            Language::English => "english",
            Language::Auto => "auto",
        }
    }

    /// Language code handed to Whisper. `None` means the engine must detect (restricted to Tamil/English).
    pub fn whisper_code(&self) -> Option<&'static str> {
        match self {
            Language::Tamil => Some("ta"),
            Language::English => Some("en"),
            Language::Auto => None,
        }
    }
}

/// Initial prompt for a concrete Whisper language code. It biases the decoder toward the right
/// script, spelling and punctuation. It contains no names or facts that could leak into output.
pub fn prompt_for(code: &str) -> &'static str {
    match code {
        "ta" => "வணக்கம். நீங்கள் எப்படி இருக்கிறீர்கள்? நான் நன்றாக இருக்கிறேன், நன்றி.",
        _ => "Hello. How are you today? I am doing well, thank you.",
    }
}

/// Whitespace-separated word count (Tamil and English are both space-delimited).
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes() {
        assert_eq!(Language::parse("Tamil").whisper_code(), Some("ta"));
        assert_eq!(Language::parse("EN").whisper_code(), Some("en"));
        assert_eq!(Language::parse("auto").whisper_code(), None);
    }

    #[test]
    fn retired_modes_fall_back_to_auto() {
        assert_eq!(Language::parse("tanglish"), Language::Auto);
        assert_eq!(Language::parse("mixed"), Language::Auto);
        assert_eq!(Language::parse(""), Language::Auto);
        assert_eq!(Language::Auto.id(), "auto");
    }

    #[test]
    fn prompts_match_script() {
        assert!(prompt_for("ta").contains("வணக்கம்"));
        assert!(prompt_for("en").starts_with("Hello"));
    }

    #[test]
    fn words() {
        assert_eq!(count_words("  hello   வணக்கம் world "), 3);
        assert_eq!(count_words(""), 0);
    }
}
