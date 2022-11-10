use crossbeam::channel;
use std::{
    env,
    fs::File,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use arrayvec::ArrayVec;
use audiobook_chapterizer::gimme_audio;
use itertools::Itertools;
use std::io::Write;
use vosk::{Alternative, CompleteResult, CompleteResultMultiple, Model, Recognizer};

const SAMPLES_BUFFER_SIZE: usize = 8 * 1024; // 8 kb

const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

// TODO: find a way to parallelize the workload

// TODO: create function that returns iterator of recognition results

// Alternative has borrowed data which relies on a recognizer.
// We serialize the data before passing it between threads to work around this.
fn serialize_alternative(result: &Alternative) -> String {
    serde_json::to_string(result).expect("json serialization should not fail")
}

fn format_duration(duration: &Option<Duration>) -> String {
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

/// Attempts to find the start of a chapter in a candidate prediction result
fn get_chapter_start<'a>(candidate: &'a CompleteResultMultiple) -> Option<&'a Alternative<'a>> {
    candidate.alternatives.iter().find(|alt| {
        // The first word in the predicted sentence should be "chapter", to reduce false positives
        // of the word appearing in the middle of a sentence
        alt.result
            .first()
            .map(|r| r.word == "chapter")
            .unwrap_or(false)
    })
}

fn main() {
    let mut args = env::args();
    args.next();

    let model_path = args.next().expect("A model path was not provided");
    let audio_file_path = args
        .next()
        .expect("A path for the audio file to be read was not provided");

    let ap = gimme_audio(&audio_file_path);
    let num_channels = 1;
    let sample_rate = ap.sample_rate();
    let total_duration = ap.total_duration();

    let calc_progress_in_secs = move |current_samples: u64| {
        current_samples as f32 / sample_rate as f32 / num_channels as f32
    };

    let model = Model::new(model_path).expect("Could not create the model");
    let mut recognizer =
        Recognizer::new(&model, sample_rate as f32).expect("Could not create the recognizer");

    recognizer.set_max_alternatives(3);
    recognizer.set_words(true);
    recognizer.set_partial_words(false);

    let start_time = Instant::now();

    let (rw_tx, rw_rx) = channel::unbounded::<String>();

    let result_writer_handle = thread::spawn(move || {
        let mut file = File::create("output.jsonl").expect("failed to create output file");
        while let Ok(msg) = rw_rx.recv() {
            eprintln!("Writing {} bytes to output file", msg.len());
            file.write_all((format!("{}\n", msg)).as_bytes())
                .expect("failed to write buffer to output file");
        }
    });

    let mut buffer: ArrayVec<i16, SAMPLES_BUFFER_SIZE> = ArrayVec::new();
    // TODO: is there a faster way to keep reading the samples into a buffer?

    let total_samples = Arc::new(AtomicU64::new(0));

    let (pp_stop_tx, pp_stop_rx) = channel::unbounded::<()>();
    let total_samples_2 = total_samples.clone();
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
            let current_samples = total_samples_2.load(Ordering::SeqCst);

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

            eprintln!(
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

    let process_result = |result: CompleteResult| {
        let multi = result.multiple().unwrap();
        if let Some(alt) = get_chapter_start(&multi) {
            rw_tx.send(serialize_alternative(alt)).unwrap();
        }
    };

    for chunk in ap.into_iter().chunks(SAMPLES_BUFFER_SIZE).into_iter() {
        for sample in chunk {
            buffer.push(sample);
            total_samples.store(total_samples.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
        }

        if let vosk::DecodingState::Finalized = recognizer.accept_waveform(&buffer) {
            process_result(recognizer.result());
        }

        buffer.clear();
    }

    process_result(recognizer.final_result());
    pp_stop_tx.send(()).unwrap();
    drop(rw_tx);
    drop(pp_stop_tx);
    result_writer_handle.join().unwrap();
    progress_printer_handle.join().unwrap();

    let end_time = Instant::now();
    let secs_processed = calc_progress_in_secs(total_samples.load(Ordering::SeqCst));
    let time_elasped = end_time - start_time;
    eprintln!(
        "Processed {:.2} seconds of audio in {:.2} seconds ({:.2}x RT)",
        secs_processed,
        time_elasped.as_secs_f32(),
        secs_processed / time_elasped.as_secs_f32()
    )
}
