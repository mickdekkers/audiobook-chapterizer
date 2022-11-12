use std::{collections::VecDeque, path::Path, time::Duration};

use audio_provider::AudioProvider;
use color_eyre::eyre::{self, Context};
use itertools::Itertools;
use lazy_static::lazy_static;
use ordered_float::NotNan;
use text2num::{
    rewrite_numbers,
    word_to_digit::{find_numbers_iter, Replace},
    Language, Token,
};
use vosk::{Alternative, WordInAlternative};

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

lazy_static! {
    static ref LANG_EN: Language = Language::english();
}

#[derive(Clone, Debug)]
pub struct OwnedWord {
    /// Time in seconds when the word starts.
    pub start: f32,

    /// Time in seconds when the word ends.
    pub end: f32,

    /// The transcribed word.
    pub word: String,

    /// Indicates that this OwnedWord replaced other OwnedWord(s)
    pub is_replacement: bool,
}

impl OwnedWord {
    pub fn is_chapter_token(&self) -> bool {
        self.word == "chapter" || self.word == "chapters"
    }
}

impl<'a> From<&'a WordInAlternative<'a>> for OwnedWord {
    fn from(wia: &'a WordInAlternative<'a>) -> Self {
        Self {
            start: wia.start,
            end: wia.end,
            word: wia.word.into(),
            is_replacement: false,
        }
    }
}

// TODO: parse number homophones
impl Token for &'_ OwnedWord {
    fn text(&self) -> &str {
        &self.word
    }

    fn text_lowercase(&self) -> String {
        self.word.to_lowercase()
    }

    fn nt_separated(&self, previous: &Self) -> bool {
        // if there is a voice pause of more than 200ms between words, we can assume that they are
        // not part of a single number
        self.start - previous.end > 0.2f32
    }
}

impl Replace for OwnedWord {
    fn replace<I: Iterator<Item = Self>>(replaced: I, data: String) -> Self {
        let mut replaced = replaced;
        let start_word = replaced.next().unwrap();
        let end = replaced
            .last()
            .map(|x| x.end)
            .unwrap_or_else(|| start_word.end);
        OwnedWord {
            start: start_word.start,
            end,
            word: data,
            is_replacement: true,
        }
    }
}

const MIN_VOCAL_PAUSE_BEFORE_CHAPTER: f32 = 0.25;

pub fn parse_chapter(match_buffer: &MatchBuffer) -> Option<Vec<OwnedWord>> {
    log::debug!("Parsing chapter with match buffer:\n{:#?}", match_buffer);
    let mut token_iter = match_buffer.iter().cloned().peekable();
    let match_token = token_iter.peek().unwrap();
    assert!(match_token.is_chapter_token());

    if let Some(prev_token) = match_buffer.token_before_match.as_ref() {
        let vocal_pause_len = match_token.start - prev_token.end;
        if vocal_pause_len < MIN_VOCAL_PAUSE_BEFORE_CHAPTER {
            log::debug!(
                "Rejecting match: vocal pause before chapter not long enough at {:.3}s",
                vocal_pause_len
            );
            return None;
        }
    }

    let tokens = token_iter.collect::<Vec<_>>();

    // Sanity check
    for token in &tokens {
        assert!(!token.is_replacement);
    }

    let mut tokens = rewrite_numbers(tokens, &*LANG_EN, 0.0);

    tokens.drain(match tokens.get(1) {
        // Keep the first two words if the second word is a number
        Some(second_token) if second_token.is_replacement => 2..,
        // Otherwise, this is not a valid chapter
        Some(second_token) => {
            log::debug!(
                "Rejecting match: token after chapter is not a number: {:#?}",
                second_token
            );
            return None;
        }
        None => {
            log::debug!("Rejecting match: no token after chapter");
            return None;
        }
    });

    Some(tokens)
}

// TODO: refactor/deduplicate this
pub fn is_chapter_token<'a>(wia: &'a WordInAlternative<'a>) -> bool {
    wia.word == "chapter" || wia.word == "chapters"
}

pub fn alt_contains_potential_match<'a>(alt: &'a Alternative<'a>) -> bool {
    alt.result.iter().any(is_chapter_token)
}

/// Given several Alternatives, returns "best" one according to several criteria.
pub fn get_best_alt<'a>(alts: &'a [Alternative<'a>]) -> &'a Alternative<'a> {
    let mut pot_matches = alts
        .iter()
        .filter(|alt| alt_contains_potential_match(alt))
        .collect::<Vec<_>>();

    // If this set of Alternatives does not contain any potential matches, just return the highest
    // confidence Alternative (the first one, since they're sorted by confidence)
    if pot_matches.is_empty() {
        return alts.get(0).expect("expected at least 1 Alternative");
    }

    let score_alt = |alt: &Alternative| {
        // Prefer higher confidence
        let mut score = alt.confidence;

        let (chap_index, chap_word) = alt
            .result
            .iter()
            .find_position(|wia| is_chapter_token(wia))
            .unwrap();

        // Slightly prefer "chapter" over "chapters"
        if chap_word.word == "chapter" {
            score += 1.0;
        } else if chap_word.word == "chapters" {
            score += 0.9;
        } else {
            unreachable!()
        }

        let following_words = alt
            .result
            .iter()
            .skip(chap_index + 1)
            .map(OwnedWord::from)
            .collect::<Vec<_>>();

        if let Some(occ) = find_numbers_iter(following_words.iter(), &*LANG_EN, 0.0).next() {
            // Only consider the number if it's right after the chapter word
            if occ.start == 0 {
                log::trace!("Occ after chapter word: {:#?}", occ);
                // The more words it was successfully able to parse into a number, the better
                score += occ.text.split(' ').count() as f32;
            } else {
                log::trace!("Occ NOT after chapter word: {:#?}", occ);
            }
        }

        NotNan::new(score).unwrap()
    };

    pot_matches.sort_by_cached_key(|alt| score_alt(alt));
    // Return the highest scoring Alternative
    pot_matches.last().unwrap()
}

pub struct FixedVecDeque<T> {
    inner: VecDeque<T>,
    max_len: usize,
}

impl<T> FixedVecDeque<T> {
    pub fn with_max_len(max_len: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(max_len),
            max_len,
        }
    }

    /// Appends an element to the back of the deque.
    /// Pops the front element if the maximum length was reached, returning it as Some(T).
    pub fn push_back(&mut self, value: T) -> Option<T> {
        let popped = if self.inner.len() == self.max_len {
            self.inner.pop_front()
        } else {
            None
        };
        self.inner.push_back(value);
        popped
    }
}

impl<T> std::ops::Deref for FixedVecDeque<T> {
    type Target = VecDeque<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> std::ops::DerefMut for FixedVecDeque<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[derive(Debug)]
pub struct MatchBuffer {
    pub token_before_match: Option<OwnedWord>,
    inner: Vec<OwnedWord>,
    capacity: usize,
}

impl MatchBuffer {
    pub fn new(post_match_context: usize) -> Self {
        let capacity = 1 + post_match_context;
        Self {
            token_before_match: None,
            inner: Vec::with_capacity(capacity),
            capacity,
        }
    }

    pub fn clear(&mut self) {
        self.inner.clear();
        self.token_before_match.take();
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn has_data(&self) -> bool {
        !self.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.inner.len() == self.capacity
    }

    pub fn iter(&self) -> impl Iterator<Item = &OwnedWord> + '_ {
        self.inner.iter()
    }

    pub fn set_token_before_match(&mut self, token: OwnedWord) {
        self.token_before_match.replace(token);
    }

    /// Tries to insert the item into the buffer. If the buffer is full, the item is not inserted
    /// and returned as Some(item)
    #[must_use]
    pub fn try_insert(&mut self, item: OwnedWord) -> Option<OwnedWord> {
        if self.is_full() {
            Some(item)
        } else {
            if self.is_empty() {
                assert!(item.is_chapter_token());
            }
            self.inner.push(item);
            None
        }
    }
}
