use text2num::word_to_digit;
use vosk::WordInAlternative;

#[derive(Clone, Debug)]
pub struct Token {
    /// Time in seconds when the word starts.
    pub start: f32,

    /// Time in seconds when the word ends.
    pub end: f32,

    /// The transcribed word.
    pub word: String,

    /// Indicates that this Token replaced other Token(s)
    pub is_replacement: bool,
}

impl Token {
    pub fn is_chapter_token(&self) -> bool {
        self.word == "chapter" || self.word == "chapters"
    }
}

impl<'a> From<&'a WordInAlternative<'a>> for Token {
    fn from(wia: &'a WordInAlternative<'a>) -> Self {
        Self {
            start: wia.start,
            end: wia.end,
            word: wia.word.into(),
            is_replacement: false,
        }
    }
}

// TODO: parse number homophones (but log when doing this too)
impl text2num::Token for &'_ Token {
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

impl word_to_digit::Replace for Token {
    fn replace<I: Iterator<Item = Self>>(replaced: I, data: String) -> Self {
        let mut replaced = replaced;
        let start_word = replaced.next().unwrap();
        let end = replaced
            .last()
            .map(|x| x.end)
            .unwrap_or_else(|| start_word.end);
        Token {
            start: start_word.start,
            end,
            word: data,
            is_replacement: true,
        }
    }
}

// TODO: refactor/deduplicate this
pub fn is_chapter_token<'a>(wia: &'a WordInAlternative<'a>) -> bool {
    wia.word == "chapter" || wia.word == "chapters"
}
