use crate::{
    audio_provider::AudioProvider,
    chapterize::{
        results_parser::{alt_contains_potential_match, ParseResult, ResultsParser},
        token::Token,
    },
    cue::CueWriter,
    fixed_vec_deque::FixedVecDeque,
    format_duration,
};
use arrayvec::ArrayVec;
use color_eyre::eyre::{self, Context, ContextCompat};
use crossbeam::channel;
use itertools::Itertools;
use std::io::Write;
use std::path::Path;
use std::{
    fs::File,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use vosk::{CompleteResult, CompleteResultMultiple, Model, Recognizer};

mod results_parser;
mod token;

const SAMPLES_BUFFER_SIZE: usize = 8 * 1024; // 8 kb

/// The number of results before and after a potential match to include as context when writing
/// potential matches to file.
const WRITE_POT_MATCH_CONTEXT: usize = 2;

/// 30 tokens should be plenty to capture the chapter number followed by most chapter titles
const POST_CHAPTER_CONTEXT: usize = 30;

const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

/// Use the average speed factor of the last 5 minutes to calculate the ETA
const ETA_CALC_WINDOW: usize = 300 / PROGRESS_INTERVAL.as_secs() as usize;

/// This margin is subtracted from the start timestamp of a chapter when output.
const PRE_CHAPTER_START_MARGIN: Duration = Duration::from_secs(1);

pub fn gimme_audio<P>(path: P) -> eyre::Result<AudioProvider>
where
    P: AsRef<Path>,
{
    // Open the media source.
    let src = std::fs::File::open(&path).wrap_err("Failed to open audio file")?;

    AudioProvider::new(src)
}

pub struct ChapterizeOptions {
    /// The path to the Vosk ASR model directory to use.
    pub model_dir_path: PathBuf,
    /// Optionally, a path to a file to write matching recognition results to.
    pub matches_file_path: Option<PathBuf>,
    /// The path to the audio file to chapterize.
    pub audio_file_path: PathBuf,
    /// The path that the output .cue file will be written to.
    pub cue_file_path: PathBuf,
}

pub fn chapterize(options: &ChapterizeOptions) -> Result<(), eyre::Error> {
    let ap = gimme_audio(&options.audio_file_path)?;
    let num_channels = 1;
    let sample_rate = ap.sample_rate();
    let total_duration = ap.total_duration();

    let calc_progress_in_secs = move |current_samples: u64| {
        current_samples as f32 / sample_rate as f32 / num_channels as f32
    };

    let model = Model::new(options.model_dir_path.to_string_lossy())
        .wrap_err("Failed to load the model")?;
    let mut recognizer =
        Recognizer::new(&model, sample_rate as f32).wrap_err("Failed to create the recognizer")?;

    recognizer.set_max_alternatives(3);
    recognizer.set_words(true);
    recognizer.set_partial_words(false);

    let start_time = chrono::Local::now();

    let (result_processor_tx, result_processor_rx) = channel::unbounded::<String>();
    let mut matches_file = match &options.matches_file_path {
        Some(matches_file_path) => {
            Some(File::create(matches_file_path).wrap_err("Failed to create matches file")?)
        }
        None => None,
    };
    let audio_file_path = options.audio_file_path.clone();

    let cue_file = File::create(&options.cue_file_path).wrap_err("Failed to create cue file")?;
    let result_processor_handle = thread::spawn(move || {
        let mut write_json_to_matches_file = |json: &str| match &mut matches_file {
            Some(matches_file) => {
                log::trace!("Writing {} bytes to matches file", json.len());
                matches_file
                    .write_all((format!("{}\n", json)).as_bytes())
                    .expect("Failed to write buffer to matches file");
            }
            None => {
                log::trace!("No matches file specified, skipped writing");
            }
        };

        let (mut results_parser, parse_result_rx) = ResultsParser::new(POST_CHAPTER_CONTEXT);

        // TODO: refactor parse result processing into trait + struct impl for .cue
        let parse_result_processor_handle = thread::spawn(move || {
            let mut cue_writer = CueWriter::new(Box::new(cue_file));

            cue_writer.write_header(&audio_file_path).unwrap();

            cue_writer
                .write_track(&Duration::ZERO, "Chapter 00")
                .unwrap();

            while let Ok(parse_result) = parse_result_rx.recv() {
                // TODO: filter out duplicate chapters
                let parsed_chapter = match parse_result {
                    ParseResult::Match(parsed_chapter) => parsed_chapter,
                    ParseResult::Failure => continue,
                    ParseResult::Incomplete => {
                        unreachable!("Incomplete results should never be sent")
                    }
                };

                let chapter_title = parsed_chapter.iter().map(|w| w.word.to_string()).join(" ");
                let chapter_start_duration =
                    Duration::from_secs_f32(parsed_chapter.get(0).unwrap().start);

                log::info!(
                    "Found chapter: {} at {}",
                    chapter_title,
                    format_duration(&Some(chapter_start_duration))
                );

                cue_writer
                    .write_track(
                        &(chapter_start_duration.saturating_sub(PRE_CHAPTER_START_MARGIN)),
                        &format!(
                            "Chapter {:02}",
                            parsed_chapter.get(1).unwrap().word.parse::<f32>().unwrap()
                        ),
                    )
                    .unwrap();
            }
        });

        let mut result_index = 0u64;
        let mut previous_results: FixedVecDeque<String> =
            FixedVecDeque::with_max_len(WRITE_POT_MATCH_CONTEXT);
        let mut last_potential_match_index: Option<u64> = None;

        let mut last_token: Option<Token> = None;
        while let Ok(msg) = result_processor_rx.recv() {
            let multi: CompleteResultMultiple = serde_json::from_str(&msg).unwrap();

            if multi.alternatives.iter().any(alt_contains_potential_match) {
                // Write previous N results as context
                for prev_result in previous_results.iter().take(WRITE_POT_MATCH_CONTEXT) {
                    write_json_to_matches_file(prev_result);
                }
                // Write potential match result
                write_json_to_matches_file(&msg);

                last_potential_match_index.replace(result_index);
            } else if let Some(lpmi) = last_potential_match_index {
                // Write next N results following a potential match as context
                if (result_index - lpmi) <= WRITE_POT_MATCH_CONTEXT as u64 {
                    write_json_to_matches_file(&msg);
                }
            }

            results_parser.ingest_results(&mut last_token, &multi);

            previous_results.push_back(msg);
            result_index += 1;
        }

        results_parser.flush();
        parse_result_processor_handle.join().unwrap();
    });

    assert!(ETA_CALC_WINDOW > 0);
    let total_samples = Arc::new(AtomicU64::new(0));
    let (progress_reporter_stop_tx, progress_reporter_stop_rx) = channel::unbounded::<()>();
    let total_samples_clone = total_samples.clone();
    let progress_reporter_handle = thread::spawn(move || {
        let mut speed_factors: FixedVecDeque<f32> = FixedVecDeque::with_max_len(ETA_CALC_WINDOW);
        let mut last_time = chrono::Local::now();
        let mut last_samples = 0u64;
        loop {
            // Wait at most PROGRESS_INTERVAL for a stop message
            match progress_reporter_stop_rx.recv_timeout(PROGRESS_INTERVAL) {
                Ok(_) => break,
                Err(err) if matches!(err, channel::RecvTimeoutError::Disconnected) => break,
                _ => (),
            }
            let current_time = chrono::Local::now();
            let current_samples = total_samples_clone.load(Ordering::SeqCst);

            let time_delta = (current_time - last_time).to_std().unwrap();
            let processed_duration =
                Duration::from_secs_f32(calc_progress_in_secs(current_samples));
            let processed_duration_delta =
                Duration::from_secs_f32(calc_progress_in_secs(current_samples - last_samples));
            let progress_percent = total_duration.map(|td| {
                f32::min(
                    f32::max(
                        processed_duration.as_secs_f32() / td.as_secs_f32() * 100.0,
                        0.0,
                    ),
                    100.0,
                )
            });
            let speed_factor = processed_duration_delta.as_secs_f32() / time_delta.as_secs_f32();
            speed_factors.push_back(speed_factor);

            let avg_speed_factor =
                speed_factors.iter().copied().sum::<f32>() / speed_factors.len() as f32;

            let (remaining_wall_time, eta) = match total_duration {
                Some(total_duration) => {
                    let remaining_to_process = total_duration - processed_duration;
                    let remaining_wall_time = Duration::from_secs_f32(
                        remaining_to_process.as_secs_f32() / avg_speed_factor,
                    );
                    let eta = current_time.checked_add_signed(
                        chrono::Duration::from_std(remaining_wall_time).unwrap(),
                    );
                    (Some(remaining_wall_time), eta)
                }
                None => (None, None),
            };

            log::info!(
                "Progress: {} @ {} of {}\tSpeed: {:.2}x\tTime left: {}\tETA: {})",
                match progress_percent {
                    Some(pct) => format!("{:05.2}%", pct),
                    None => "??%".into(),
                },
                format_duration(&Some(processed_duration)),
                format_duration(&total_duration),
                speed_factor,
                format_duration(&remaining_wall_time),
                match eta {
                    Some(eta) => eta.format("%a %e %b %Y %T").to_string(),
                    None => "??".into(),
                }
            );

            last_time = current_time;
            last_samples = current_samples;
        }
    });

    let total_samples_clone = total_samples.clone();
    let asr_handle = thread::spawn(move || {
        let process_result = |result: CompleteResult| {
            let multi = result.multiple().unwrap();
            // The prediction result contains borrowed data which depends on the recognizer.
            // We serialize the data before passing it between threads to work around this.
            let msg = serde_json::to_string(&multi).unwrap();
            result_processor_tx.send(msg).unwrap();
        };

        let mut buffer: ArrayVec<i16, SAMPLES_BUFFER_SIZE> = ArrayVec::new();
        // TODO: is there a faster way to keep reading the samples into a buffer?
        for chunk in ap.into_iter().chunks(SAMPLES_BUFFER_SIZE).into_iter() {
            let mut chunk_size = 0usize;
            for sample in chunk {
                buffer.push(sample);
                chunk_size += 1;
            }
            total_samples_clone.store(
                total_samples_clone.load(Ordering::SeqCst) + chunk_size as u64,
                Ordering::SeqCst,
            );

            if let vosk::DecodingState::Finalized = recognizer.accept_waveform(&buffer) {
                process_result(recognizer.result());
            }

            buffer.clear();
        }
        process_result(recognizer.final_result());
        progress_reporter_stop_tx.send(()).unwrap();
    });

    asr_handle.join().unwrap();
    result_processor_handle.join().unwrap();
    progress_reporter_handle.join().unwrap();

    let end_time = chrono::Local::now();
    let secs_processed = calc_progress_in_secs(total_samples.load(Ordering::SeqCst));
    let time_elasped = (end_time - start_time).to_std().unwrap();
    log::info!(
        "Processed {:.2} seconds of audio in {:.2} seconds ({:.2}x RT)",
        secs_processed,
        time_elasped.as_secs_f32(),
        secs_processed / time_elasped.as_secs_f32()
    );

    Ok(())
}
