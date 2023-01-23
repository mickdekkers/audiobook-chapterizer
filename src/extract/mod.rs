use self::ffprobe::ffprobe;
use crate::{cue::CueWriter, format_duration};
use color_eyre::{eyre::Context, Result};
use std::{fs::File, path::Path};

mod ffprobe;

pub fn extract_chapters(
    audio_file_path: impl AsRef<Path>,
    cue_file_path: impl AsRef<Path>,
) -> Result<bool> {
    let chapters = ffprobe(audio_file_path.as_ref())?.chapters;
    if chapters.is_empty() {
        log::debug!("Metadata contains no chapters");
        return Ok(false);
    }

    let cue_file = File::create(&cue_file_path).wrap_err("Failed to create cue file")?;
    let mut cue_writer = CueWriter::new(Box::new(cue_file));

    cue_writer.write_header(audio_file_path.as_ref())?;

    for chapter in chapters {
        let title = chapter.title().unwrap_or("Untitled");

        log::debug!(
            "Extracted chapter {} @ {}: \"{}\"",
            chapter.id,
            format_duration(&Some(chapter.start())),
            title
        );

        cue_writer.write_track(&chapter.start(), title)?;
    }

    Ok(true)
}
