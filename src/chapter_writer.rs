use std::time::Duration;

use color_eyre::eyre;

pub trait ChapterWriter {
    fn on_chapter_start(&mut self, start_time: &Duration, title: &str) -> eyre::Result<()>;

    fn on_end_of_file(&mut self, file_duration: &Duration) -> eyre::Result<()>;
}
