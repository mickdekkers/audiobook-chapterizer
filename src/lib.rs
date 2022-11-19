use std::time::Duration;

pub mod audio_provider;
pub mod chapterize;
pub mod cue;
pub mod fixed_vec_deque;

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
