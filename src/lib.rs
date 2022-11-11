use std::{path::Path, time::Duration};

use audio_provider::AudioProvider;
use color_eyre::eyre::{self, Context};
use lazy_static::lazy_static;
use text2num::{rewrite_numbers, word_to_digit::Replace, Language, Token};
use vosk::{Alternative, CompleteResultMultiple, WordInAlternative};

mod audio_provider;

pub fn gimme_audio<P>(path: P) -> eyre::Result<AudioProvider>
where
    P: AsRef<Path>,
{
    // Open the media source.
    let src = std::fs::File::open(&path).wrap_err("Failed to open audio file")?;

    AudioProvider::new(src)
}

pub fn format_duration(duration: &Option<Duration>) -> String {
    let duration = match duration {
        Some(duration) => duration,
        None => return "??".into(),
    };

    let millis = duration.subsec_millis();
    let seconds = duration.as_secs() % 60;
    let minutes = (duration.as_secs() / 60) % 60;
    let hours = (duration.as_secs() / 60) / 60;

    format!(
        "{:02}:{:02}:{:02}.{:02}",
        hours,
        minutes,
        seconds,
        millis / 10
    )
}

/// There are 75 frames in one second
const CUE_FRAMES_PER_SECOND: f32 = 75.0;

pub fn duration_to_cue_index(duration: &Duration) -> String {
    let frames = (duration.subsec_millis() as f32 / 1000.0 * CUE_FRAMES_PER_SECOND) as u32;
    let seconds = duration.as_secs() % 60;
    let minutes = duration.as_secs() / 60; // integer divison, no need to floor

    format!("{}:{}:{:02}", minutes, seconds, frames)
}

/// Attempts to find the start of a chapter in a candidate prediction result
pub fn get_chapter_start<'a>(candidate: &'a CompleteResultMultiple) -> Option<&'a Alternative<'a>> {
    candidate.alternatives.iter().find(|alt| {
        // The first word in the predicted sentence should be "chapter", to reduce false positives
        // of the word appearing in the middle of a sentence
        alt.result
            .first()
            .map(|r| r.word == "chapter")
            .unwrap_or(false)
    })
}

lazy_static! {
    static ref LANG_EN: Language = Language::english();
}

#[derive(Clone, Debug)]
pub struct ReplacedWord {
    /// Time in seconds when the word starts.
    pub start: f32,

    /// Time in seconds when the word ends.
    pub end: f32,

    /// The transcribed word.
    pub word: String,
}

#[derive(Clone, Debug)]
pub enum DecodedWord<'a> {
    Wia(&'a WordInAlternative<'a>),
    Replaced(ReplacedWord),
}

impl<'a> DecodedWord<'a> {
    pub fn word(&self) -> &str {
        match self {
            DecodedWord::Wia(wia) => wia.word,
            DecodedWord::Replaced(repl) => repl.word.as_str(),
        }
    }

    pub fn start(&self) -> f32 {
        match self {
            DecodedWord::Wia(wia) => wia.start,
            DecodedWord::Replaced(repl) => repl.start,
        }
    }

    pub fn end(&self) -> f32 {
        match self {
            DecodedWord::Wia(wia) => wia.end,
            DecodedWord::Replaced(repl) => repl.end,
        }
    }
}

impl Token for &'_ DecodedWord<'_> {
    fn text(&self) -> &str {
        self.word()
    }

    fn text_lowercase(&self) -> String {
        self.word().to_lowercase()
    }

    fn nt_separated(&self, previous: &Self) -> bool {
        // if there is a voice pause of more than 200ms between words, we can assume that they are
        // not part of a single number
        self.start() - previous.end() > 0.2f32
    }
}

impl Replace for DecodedWord<'_> {
    fn replace<I: Iterator<Item = Self>>(replaced: I, data: String) -> Self {
        let mut replaced = replaced;
        let start_word = replaced.next().unwrap();
        let end = replaced
            .last()
            .map(|x| x.end())
            .unwrap_or_else(|| start_word.end());
        Self::Replaced(ReplacedWord {
            start: start_word.start(),
            end,
            word: data,
        })
    }
}

impl<'a> From<&'a WordInAlternative<'a>> for DecodedWord<'a> {
    fn from(wia: &'a WordInAlternative<'a>) -> Self {
        Self::Wia(wia)
    }
}

pub fn parse_chapter<'a>(alt: &'a Alternative) -> Vec<DecodedWord<'a>> {
    let tokens = alt
        .result
        .iter()
        .map(Into::into)
        .collect::<Vec<DecodedWord<'_>>>();

    let mut res = rewrite_numbers(tokens, &*LANG_EN, 0.0);

    let chapter_word = res.get(0).unwrap();

    // Sanity check
    assert_eq!(chapter_word.word(), "chapter");

    res.drain(match res.get(1) {
        // Keep the first two words if the second word is a number
        Some(word) if matches!(word, DecodedWord::Replaced(_)) => 2..,
        // Keep only the first word otherwise
        Some(_) | None => 1..,
    });

    res
}
