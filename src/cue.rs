use lazy_static::lazy_static;
use regex::Regex;
use std::{io::Write, path::Path, time::Duration};

use color_eyre::eyre::{self, eyre, Context};

/// There are 75 frames in one second
const CUE_FRAMES_PER_SECOND: f32 = 75.0;

pub fn duration_to_cue_index(duration: &Duration) -> String {
    let frames = (duration.subsec_millis() as f32 / 1000.0 * CUE_FRAMES_PER_SECOND) as u32;
    let seconds = duration.as_secs() % 60;
    let minutes = duration.as_secs() / 60; // integer divison, no need to floor

    format!("{}:{}:{:02}", minutes, seconds, frames)
}

pub struct CueWriter {
    writer: Box<dyn Write>,
    track_num: usize,
    header_written: bool,
}

// TODO: double check encoding, is ASCII required or is UTF8 ok?
impl CueWriter {
    pub fn new(writer: Box<dyn Write>) -> Self {
        Self {
            writer,
            track_num: 1,
            header_written: false,
        }
    }

    fn sanitize_string<T: AsRef<str>>(s: T) -> String {
        lazy_static! {
            static ref SANITIZE_STRING_REGEX: Regex = Regex::new("[\r\n\"\\\\]+").unwrap();
        }

        SANITIZE_STRING_REGEX
            .replace_all(s.as_ref(), "")
            .trim()
            .to_string()
    }

    pub fn write_header(&mut self, audio_file_path: &Path) -> eyre::Result<()> {
        if self.header_written {
            return Err(eyre!("Failed to write cue header: header already written"));
        }

        let file_name = audio_file_path.file_name().unwrap().to_string_lossy();
        let file_type = match audio_file_path.extension() {
            Some(ext) => match &ext.to_string_lossy().to_lowercase().as_str() {
                // https://wiki.hydrogenaud.io/index.php?title=Cue_sheet#Most_often_used
                &"mp3" => "MP3",
                &"wav" | &"wv" | &"flac" => "WAVE",
                &"m4a" | &"m4b" => "MP4",
                _ => "BINARY",
            },
            None => "BINARY",
        };

        let cue_header = &format!(
            "FILE \"{}\" {}",
            &Self::sanitize_string(&file_name),
            file_type
        );

        self.writer
            .write_all((format!("{}\n", cue_header)).as_bytes())
            .wrap_err("Failed to write cue header")?;

        self.header_written = true;

        Ok(())
    }

    pub fn write_track(&mut self, duration: &Duration, title: &str) -> eyre::Result<()> {
        if !self.header_written {
            return Err(eyre!("Failed to write cue track: must write header first"));
        }

        let cue_track = unindent::unindent(&format!(
            "
                TRACK {} AUDIO
                    TITLE \"{}\"
                    INDEX 01 {}
            ",
            self.track_num,
            &Self::sanitize_string(title),
            duration_to_cue_index(duration),
        ));

        self.writer
            .write_all(cue_track.as_bytes())
            .wrap_err("Failed to write cue track")?;

        self.track_num += 1;

        Ok(())
    }
}
