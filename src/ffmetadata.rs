use lazy_static::lazy_static;
use regex::Regex;
use std::{io::Write, time::Duration};

use color_eyre::eyre::{self, eyre, Context};

use crate::chapter_writer::ChapterWriter;

pub struct FfmetadataWriter {
    writer: Box<dyn Write>,
    header_written: bool,
    /// A tuple of (start_time, title). We still need the end time to actually write the chapter.
    partial_chapter: Option<(Duration, String)>,
}

impl FfmetadataWriter {
    pub fn new(writer: Box<dyn Write>) -> Self {
        Self {
            writer,
            header_written: false,
            partial_chapter: None,
        }
    }

    // ffmpeg docs 22.9: Metadata keys or values containing special characters (‘=’, ‘;’, ‘#’, ‘\’ and a newline) must be escaped with a backslash ‘\’.
    fn sanitize_string<T: AsRef<str>>(s: T) -> String {
        lazy_static! {
            static ref CR_REGEX: Regex = Regex::new("\r+").unwrap();
            static ref SPECIAL_CHARS_REGEX: Regex = Regex::new("[\n=;#\\\\]").unwrap();
        }

        SPECIAL_CHARS_REGEX
            .replace_all(CR_REGEX.replace_all(s.as_ref(), "").as_ref(), "\\$0")
            .trim()
            .to_string()
    }

    pub fn write_header(&mut self) -> eyre::Result<()> {
        if self.header_written {
            return Err(eyre!(
                "Failed to write ffmetadata header: header already written"
            ));
        }

        self.writer
            .write_all((format!("{}\n", ";FFMETADATA1")).as_bytes())
            .wrap_err("Failed to write ffmetadata header")?;

        self.header_written = true;

        Ok(())
    }

    pub fn write_chapter(
        &mut self,
        start_time: &Duration,
        end_time: &Duration,
        title: &str,
    ) -> eyre::Result<()> {
        if !self.header_written {
            return Err(eyre!(
                "Failed to write ffmetadata chapter: must write header first"
            ));
        }

        let chapter_data = unindent::unindent(&format!(
            "
                [CHAPTER]
                TIMEBASE=1/1000
                START={}
                END={}
                title={}
            ",
            start_time.as_millis(),
            end_time.as_millis(),
            &Self::sanitize_string(title),
        ));

        self.writer
            .write_all(chapter_data.as_bytes())
            .wrap_err("Failed to write ffmetadata chapter")?;

        Ok(())
    }
}

impl ChapterWriter for FfmetadataWriter {
    fn on_chapter_start(&mut self, start_time: &Duration, title: &str) -> eyre::Result<()> {
        if let Some((prev_start_time, prev_title)) = self.partial_chapter.take() {
            self.write_chapter(&prev_start_time, start_time, &prev_title)?;
        }

        self.partial_chapter = Some((*start_time, title.to_string()));

        Ok(())
    }

    fn on_end_of_file(&mut self, file_duration: &Duration) -> eyre::Result<()> {
        if let Some((start_time, title)) = self.partial_chapter.take() {
            self.write_chapter(&start_time, file_duration, &title)?;
        }

        Ok(())
    }
}
