use std::time::Duration;

/// There are 75 frames in one second
const CUE_FRAMES_PER_SECOND: f32 = 75.0;

pub fn duration_to_cue_index(duration: &Duration) -> String {
    let frames = (duration.subsec_millis() as f32 / 1000.0 * CUE_FRAMES_PER_SECOND) as u32;
    let seconds = duration.as_secs() % 60;
    let minutes = duration.as_secs() / 60; // integer divison, no need to floor

    format!("{}:{}:{:02}", minutes, seconds, frames)
}
