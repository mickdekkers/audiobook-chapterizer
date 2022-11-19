use super::token::{is_chapter_token, Token};
use crossbeam::channel;
use itertools::Itertools;
use lazy_static::lazy_static;
use ordered_float::NotNan;
use text2num::{rewrite_numbers, word_to_digit::find_numbers_iter, Language};
use vosk::{Alternative, CompleteResultMultiple};

const MIN_VOCAL_PAUSE_BEFORE_CHAPTER: f32 = 0.25;

lazy_static! {
    static ref LANG_EN: Language = Language::english();
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
            .map(Token::from)
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

        // TODO: log how score was determined
        NotNan::new(score).unwrap()
    };

    pot_matches.sort_by_cached_key(|alt| score_alt(alt));
    // Return the highest scoring Alternative
    pot_matches.last().unwrap()
}

#[derive(Debug)]
pub enum ParseResult {
    Match(Vec<Token>),
    Incomplete,
    Failure,
}

#[derive(Debug)]
pub struct ResultsParser {
    parse_result_tx: channel::Sender<ParseResult>,
    buffer: Vec<Token>,
    capacity: usize,
}

impl ResultsParser {
    pub fn new(post_match_context: usize) -> (Self, channel::Receiver<ParseResult>) {
        let (tx, rx) = channel::unbounded();
        let capacity = 2 + post_match_context;

        (
            Self {
                buffer: Vec::with_capacity(capacity),
                capacity,
                parse_result_tx: tx,
            },
            rx,
        )
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn has_data(&self) -> bool {
        !self.is_empty()
    }

    fn is_full(&self) -> bool {
        self.buffer.len() == self.capacity
    }

    /// Ingests a batch of prediction results. Keeps the prev_token Option up to date with to the
    /// last token in the batch.
    pub fn ingest_results(
        &mut self,
        prev_token: &mut Option<Token>,
        multi: &CompleteResultMultiple,
    ) {
        let best_alt = get_best_alt(&multi.alternatives);
        let alt_token_iter = best_alt.result.iter();
        for token in alt_token_iter.map(Token::from) {
            if self.has_data() || token.is_chapter_token() {
                // If this is a new match, first push the token before the chapter token
                if self.is_empty() && token.is_chapter_token() {
                    if let Some(ref prev_token) = prev_token {
                        self.push(prev_token.clone());
                    }
                }

                self.push(token.clone());
            }
            prev_token.replace(token);
        }
    }

    /// Consumes and flushes the ResultsParser. Attempts to parse the remaining contents of the
    /// buffer before dropping.
    pub fn flush(mut self) {
        self.do_parse(true);
    }

    /// Pushes the item into the ResultsParser.
    fn push(&mut self, item: Token) {
        self.buffer.push(item);
        assert!(self.buffer.len() <= self.capacity);
        self.do_parse(false);
    }

    fn do_parse(&mut self, is_end: bool) {
        let parse_result = self.parse_chapter(is_end);

        if is_end {
            // If this is the end, parse_chapter should not return Incomplete
            assert!(!matches!(parse_result, ParseResult::Incomplete));
        }

        match parse_result {
            ParseResult::Match(_) | ParseResult::Failure => {
                self.buffer.clear();
            }
            ParseResult::Incomplete => {
                if self.is_full() {
                    log::warn!(
                        "parse_chapter returned ParseResult::Incomplete despite buffer being full!"
                    );
                    self.buffer.clear();
                }
            }
        }

        // Don't send Incomplete results
        if !matches!(parse_result, ParseResult::Incomplete) {
            self.parse_result_tx.send(parse_result).unwrap();
        }
    }

    fn parse_chapter(&self, is_end: bool) -> ParseResult {
        log::debug!("Parsing chapter with match buffer:\n{:#?}", self);

        let (chapter_token_index, chapter_token) =
            match self.buffer.iter().find_position(|t| t.is_chapter_token()) {
                Some(tuple) => tuple,
                None => {
                    return if is_end {
                        log::debug!("ParseResult::Failure: no chapter token");
                        ParseResult::Failure
                    } else {
                        log::debug!("ParseResult::Incomplete: waiting for chapter token");
                        ParseResult::Incomplete
                    }
                }
            };

        if let Some(prev_token) = chapter_token_index
            .checked_sub(1)
            .and_then(|index| self.buffer.get(index))
        {
            let vocal_pause_len = chapter_token.start - prev_token.end;
            if vocal_pause_len < MIN_VOCAL_PAUSE_BEFORE_CHAPTER {
                log::debug!(
                    "ParseResult::Failure: vocal pause before chapter token not long enough at {:.3}s",
                    vocal_pause_len
                );
                return ParseResult::Failure;
            }
        }

        if self.buffer.iter().skip(chapter_token_index + 1).count() == 0 {
            return if is_end {
                log::debug!("ParseResult::Failure: no token after chapter");
                ParseResult::Failure
            } else {
                log::debug!("ParseResult::Incomplete: waiting for token after chapter token");
                ParseResult::Incomplete
            };
        }

        let tokens = self
            .buffer
            .iter()
            .skip(chapter_token_index)
            .cloned()
            .collect::<Vec<_>>();

        // Sanity check
        for token in &tokens {
            assert!(!token.is_replacement);
        }

        let mut tokens = rewrite_numbers(tokens, &*LANG_EN, 0.0);

        let chapter_token = tokens.get(0).unwrap();

        // Sanity check
        assert!(chapter_token.is_chapter_token());
        assert!(!chapter_token.is_replacement);

        let chapter_number_token = tokens.get(1).unwrap();
        if !chapter_number_token.is_replacement {
            log::debug!(
                "ParseResult::Failure: token after chapter is not a number: {:#?}",
                chapter_number_token
            );
            return ParseResult::Failure;
        }

        let token_after_chapter_number = tokens.get(2);
        if token_after_chapter_number.is_none() && !is_end {
            // We can't yet be certain that this is the end of the number string
            log::debug!("ParseResult::Incomplete: waiting for token after chapter number token");
            return ParseResult::Incomplete;
        }

        // TODO: attempt to extract chapter title using vocal pause

        tokens.drain(2..);

        let parse_result = ParseResult::Match(tokens);
        log::debug!("ParseResult::Match: {:#?}", parse_result);
        parse_result
    }
}
