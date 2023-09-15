use self::ffprobe::ffprobe;
use crate::{
    chapter_writer::ChapterWriter, cue::CueWriter, ffmetadata::FfmetadataWriter, format_duration,
};
use color_eyre::{eyre::Context, Result};
use std::{fs::File, path::PathBuf, time::Duration};

mod ffprobe;

pub struct ExtractOptions {
    /// The path to the audio file to chapterize.
    pub audio_file_path: PathBuf,
    /// The path that the output .cue file will be written to.
    pub cue_file_path: Option<PathBuf>,
    /// The path that the output ffmetadata file will be written to.
    pub ffmetadata_file_path: Option<PathBuf>,
}

/// For some reason, ffprobe reports durations that are exactly 25 ms later than what ffmpeg
/// reports. This function serves as a workaround for that issue.
/// See https://stackoverflow.com/questions/67571358/ffmpeg-timing-metadata-values-differ-from-the-ffprobe-output
fn ffprobe_duration_difference_workaround(duration: Duration) -> Duration {
    // As a workaround, simply subtract 25 ms from the duration
    duration.saturating_sub(Duration::from_millis(25))
}

pub fn extract_chapters(options: &ExtractOptions) -> Result<bool> {
    let chapters = ffprobe(&options.audio_file_path)?.chapters;
    if chapters.is_empty() {
        log::debug!("Metadata contains no chapters");
        return Ok(false);
    }

    // TODO: dedupe/abstract chapter writers setup and usage

    let cue_file = options
        .cue_file_path
        .as_ref()
        .map(|cue_file_path| File::create(cue_file_path).wrap_err("Failed to create cue file"))
        .transpose()?;
    let ffmetadata_file = options
        .ffmetadata_file_path
        .as_ref()
        .map(|ffmetadata_file_path| {
            File::create(ffmetadata_file_path).wrap_err("Failed to create ffmetadata file")
        })
        .transpose()?;

    let mut chapter_writers = {
        let mut chapter_writers: Vec<Box<dyn ChapterWriter>> = Vec::with_capacity(2);

        if let Some(cue_file) = cue_file {
            let mut cue_writer = CueWriter::new(Box::new(cue_file));
            cue_writer.write_header(&options.audio_file_path).unwrap();
            chapter_writers.push(Box::new(cue_writer));
        }

        if let Some(ffmetadata_file) = ffmetadata_file {
            let mut ffmetadata_writer = FfmetadataWriter::new(Box::new(ffmetadata_file));
            ffmetadata_writer.write_header().unwrap();
            chapter_writers.push(Box::new(ffmetadata_writer));
        }

        chapter_writers
    };

    if chapter_writers.is_empty() {
        unreachable!("No chapter writers specified, cli args validation should have caught this");
    }

    // Ensure that the first chapter in the output starts at 0:00:00.00
    let first_chapter = chapters.first().unwrap();
    if ffprobe_duration_difference_workaround(first_chapter.start()) != Duration::ZERO {
        log::debug!("Adding 0th chapter @ 0:00:00.00");

        for chapter_writer in chapter_writers.iter_mut() {
            chapter_writer
                .on_chapter_start(Duration::ZERO, "Chapter 00")
                .unwrap();
        }
    }

    for chapter in &chapters {
        let title = chapter.title().unwrap_or("Untitled");
        let start = ffprobe_duration_difference_workaround(chapter.start());

        log::debug!(
            "Extracted chapter {} @ {}: \"{}\"",
            chapter.id,
            format_duration(&Some(start)),
            title
        );

        for chapter_writer in chapter_writers.iter_mut() {
            chapter_writer.on_chapter_start(start, title).unwrap();
        }
    }

    let last_chapter = chapters.last().unwrap();

    for chapter_writer in chapter_writers.iter_mut() {
        chapter_writer
            .on_end_of_file(ffprobe_duration_difference_workaround(last_chapter.end()))
            .unwrap();
    }

    Ok(true)
}
