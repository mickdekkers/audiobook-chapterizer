use crossbeam::channel;
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use arrayvec::ArrayVec;
use audiobook_chapterizer::{format_duration, get_chapter_start, gimme_audio, parse_chapter};
use clap::{
    builder::{OsStringValueParser, TypedValueParser},
    ArgAction, Parser,
};
use color_eyre::eyre::{self, Context, ContextCompat};
use itertools::Itertools;
use log::LevelFilter;
use std::io::Write;
use vosk::{CompleteResult, Model, Recognizer};

const SAMPLES_BUFFER_SIZE: usize = 8 * 1024; // 8 kb

const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

// TODO: find a way to parallelize the workload

// TODO: create function that returns iterator of recognition results

fn verify_jsonl_ext(os: OsString) -> Result<PathBuf, &'static str> {
    let path = PathBuf::from(os);
    if path.extension() != Some(OsStr::new("jsonl")) {
        return Err("path must end in .jsonl");
    }
    Ok(path)
}

#[derive(Parser)]
struct Cli {
    /// Makes logging more verbose. Pass once for debug log level, twice for trace log level.
    #[clap(short, action = ArgAction::Count, global = true)]
    verbose: u8,
    /// The path to the Vosk ASR model directory to use.
    #[clap(value_name = "model_dir", long = "model", default_value = "./model")]
    model_dir_path: PathBuf,
    /// Optionally, a path to a file to write matching recognition results to. The path must end in
    /// .jsonl
    #[clap(
        value_name = "matches_file",
        long = "write_matches",
        value_parser = OsStringValueParser::new().try_map(verify_jsonl_ext)
    )]
    matches_file_path: Option<PathBuf>,
    /// The path to the audio file to chapterize.
    #[clap(value_name = "audio_file", short = 'i')]
    audio_file_path: PathBuf,
    /// The path that the output .cue file will be written to.
    #[clap(value_name = "cue_file")]
    cue_file_path: PathBuf,
}

fn main() -> Result<(), eyre::Error> {
    color_eyre::install()?;
    let cli = Cli::parse();
    env_logger::Builder::new()
        .filter_level(match cli.verbose {
            0 => LevelFilter::Info,
            1 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        })
        .init();

    let ap = gimme_audio(&cli.audio_file_path)?;
    let num_channels = 1;
    let sample_rate = ap.sample_rate();
    let total_duration = ap.total_duration();

    let calc_progress_in_secs = move |current_samples: u64| {
        current_samples as f32 / sample_rate as f32 / num_channels as f32
    };

    let model =
        Model::new(cli.model_dir_path.to_string_lossy()).wrap_err("Failed to load the model")?;
    let mut recognizer =
        Recognizer::new(&model, sample_rate as f32).wrap_err("Failed to create the recognizer")?;

    recognizer.set_max_alternatives(3);
    recognizer.set_words(true);
    recognizer.set_partial_words(false);

    let start_time = Instant::now();

    let (result_processor_tx, result_processor_rx) = channel::unbounded::<String>();
    let mut matches_file = match cli.matches_file_path {
        Some(matches_file_path) => {
            Some(File::create(matches_file_path).wrap_err("Failed to create matches file")?)
        }
        None => None,
    };
    let mut cue_file = File::create(cli.cue_file_path).wrap_err("Failed to create cue file")?;
    let result_processor_handle = thread::spawn(move || {
        while let Ok(msg) = result_processor_rx.recv() {
            let multi = serde_json::from_str(&msg).unwrap();
            if let Some(alt) = get_chapter_start(&multi) {
                match &mut matches_file {
                    Some(matches_file) => {
                        let alt_json = serde_json::to_string_pretty(&alt).unwrap();
                        log::debug!("Writing {} bytes to matches file", alt_json.len());
                        matches_file
                            .write_all((format!("{}\n", alt_json)).as_bytes())
                            .expect("Failed to write buffer to matches file");
                    }
                    None => {
                        log::debug!("No matches file specified, skipped writing match");
                    }
                }

                let chapter_words = parse_chapter(alt);
                log::info!(
                    "Found chapter: {} at {}",
                    chapter_words.iter().map(|w| w.word()).join(" "),
                    format_duration(&Some(Duration::from_secs_f32(
                        chapter_words.get(0).unwrap().start()
                    )))
                )
            }
        }

        // TODO: write CUE when done
    });

    let total_samples = Arc::new(AtomicU64::new(0));

    let (progress_reporter_stop_tx, progress_reporter_stop_rx) = channel::unbounded::<()>();
    let total_samples_clone = total_samples.clone();
    let progress_reporter_handle = thread::spawn(move || {
        let mut last_time = Instant::now();
        let mut last_samples = 0u64;
        loop {
            // Wait at most PROGRESS_INTERVAL for a stop message
            match progress_reporter_stop_rx.recv_timeout(PROGRESS_INTERVAL) {
                Ok(_) => break,
                Err(err) if matches!(err, channel::RecvTimeoutError::Disconnected) => break,
                _ => (),
            }
            let current_time = Instant::now();
            let current_samples = total_samples_clone.load(Ordering::SeqCst);

            let time_delta = current_time - last_time;
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

            log::info!(
                "Progress: {} @ {} of {}, speed= {:.2}x",
                match progress_percent {
                    Some(pct) => format!("{:05.2}%", pct),
                    None => "??%".into(),
                },
                format_duration(&Some(processed_duration)),
                format_duration(&total_duration),
                processed_duration_delta.as_secs_f32() / time_delta.as_secs_f32()
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

    let end_time = Instant::now();
    let secs_processed = calc_progress_in_secs(total_samples.load(Ordering::SeqCst));
    let time_elasped = end_time - start_time;
    log::info!(
        "Processed {:.2} seconds of audio in {:.2} seconds ({:.2}x RT)",
        secs_processed,
        time_elasped.as_secs_f32(),
        secs_processed / time_elasped.as_secs_f32()
    );

    Ok(())
}
