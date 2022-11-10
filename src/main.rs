use crossbeam::channel;
use std::{
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
use audiobook_chapterizer::{
    format_duration, get_chapter_start, gimme_audio, serialize_alternative,
};
use clap::{ArgAction, Parser};
use color_eyre::eyre::{self, Context, ContextCompat};
use itertools::Itertools;
use log::LevelFilter;
use std::io::Write;
use vosk::{CompleteResult, Model, Recognizer};

const SAMPLES_BUFFER_SIZE: usize = 8 * 1024; // 8 kb

const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

// TODO: find a way to parallelize the workload

// TODO: create function that returns iterator of recognition results

#[derive(Parser)]
struct Cli {
    /// Makes logging more verbose. Pass once for debug log level, twice for trace log level.
    #[clap(short, action = ArgAction::Count, global = true)]
    verbose: u8,
    /// The path to the Vosk ASR model to use.
    #[clap(long, default_value = "./model")]
    model: PathBuf,
    /// The audio file to chapterize.
    audio_file: PathBuf,
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

    let ap = gimme_audio(&cli.audio_file)?;
    let num_channels = 1;
    let sample_rate = ap.sample_rate();
    let total_duration = ap.total_duration();

    let calc_progress_in_secs = move |current_samples: u64| {
        current_samples as f32 / sample_rate as f32 / num_channels as f32
    };

    let model = Model::new(cli.model.to_string_lossy()).wrap_err("Failed to load the model")?;
    let mut recognizer =
        Recognizer::new(&model, sample_rate as f32).wrap_err("Failed to create the recognizer")?;

    recognizer.set_max_alternatives(3);
    recognizer.set_words(true);
    recognizer.set_partial_words(false);

    let start_time = Instant::now();

    let (rw_tx, rw_rx) = channel::unbounded::<String>();
    let mut file = File::create("output.jsonl").wrap_err("Failed to create output file")?;
    let result_writer_handle = thread::spawn(move || {
        while let Ok(msg) = rw_rx.recv() {
            log::debug!("Writing {} bytes to output file", msg.len());
            file.write_all((format!("{}\n", msg)).as_bytes())
                .expect("Failed to write buffer to output file");
        }
    });

    let total_samples = Arc::new(AtomicU64::new(0));

    let (pp_stop_tx, pp_stop_rx) = channel::unbounded::<()>();
    let total_samples_clone = total_samples.clone();
    let progress_printer_handle = thread::spawn(move || {
        let mut last_time = Instant::now();
        let mut last_samples = 0u64;
        loop {
            // Wait at most PROGRESS_INTERVAL for a stop message
            match pp_stop_rx.recv_timeout(PROGRESS_INTERVAL) {
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
            if let Some(alt) = get_chapter_start(&multi) {
                rw_tx.send(serialize_alternative(alt)).unwrap();
            }
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
        pp_stop_tx.send(()).unwrap();
    });

    asr_handle.join().unwrap();
    result_writer_handle.join().unwrap();
    progress_printer_handle.join().unwrap();

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
