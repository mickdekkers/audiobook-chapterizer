use std::time::Duration;

use audio_provider::AudioProvider;
use vosk::{Alternative, CompleteResultMultiple};

mod audio_provider;

pub fn gimme_audio(path: &str) -> AudioProvider {
    // Open the media source.
    let src = std::fs::File::open(&path).expect("failed to open media");

    AudioProvider::new(src)
}

// Alternative has borrowed data which relies on a recognizer.
// We serialize the data before passing it between threads to work around this.
pub fn serialize_alternative(result: &Alternative) -> String {
    serde_json::to_string(result).expect("json serialization should not fail")
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
