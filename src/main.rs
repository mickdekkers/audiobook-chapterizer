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
use vosk::{CompleteResult, Model, Recognizer};

const SAMPLES_BUFFER_SIZE: usize = 8 * 1024; // 8 kb

const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

// TODO: find a way to parallelize the workload

// TODO: create function that returns iterator of recognition results

// CompleteResult has borrowed data which relies on a recognizer.
// We serialize the data before passing it between threads to work around this.
fn serialize_result(result: CompleteResult) -> String {
    serde_json::to_string(&result).expect("json serialization should not fail")
}

fn main() {
    let mut args = env::args();
    args.next();

    let model_path = args.next().expect("A model path was not provided");
    let audio_file_path = args
        .next()
        .expect("A path for the audio file to be read was not provided");

    // let file = std::fs::File::open(audio_file_path).expect("Could not open file");
    let ap = gimme_audio(&audio_file_path);
    // let buf_reader = std::io::BufReader::with_capacity(SAMPLES_BUFFER_SIZE * 8, file);
    // let source = Decoder::new(file).expect("Could not create the audio reader");
    // let num_channels = source.channels();
    // let sample_rate = source.sample_rate();
    let num_channels = 1;
    let sample_rate = ap.sample_rate();
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
            if !msg.contains("chapter") {
                continue;
            }
            println!("Writing {} bytes to output file", msg.len());
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

            // TODO: obtain total duration and calculate progress percentage
            // TODO: pretty format duration
            eprintln!(
                "Progress: {:.3} seconds / ??, speed= {:.2}x",
                processed_duration.as_secs_f32(),
                processed_duration_delta.as_secs_f32() / time_delta.as_secs_f32()
            );

            last_time = current_time;
            last_samples = current_samples;
        }
    });

    for chunk in ap.into_iter().chunks(SAMPLES_BUFFER_SIZE).into_iter() {
        for sample in chunk {
            // let sample = sample_result.expect("Error reading sample");
            buffer.push(sample);
            total_samples.store(total_samples.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
        }

        if let vosk::DecodingState::Finalized = recognizer.accept_waveform(&buffer) {
            rw_tx.send(serialize_result(recognizer.result())).unwrap();
        }

        buffer.clear();
    }

    rw_tx
        .send(serialize_result(recognizer.final_result()))
        .unwrap();
    pp_stop_tx.send(()).unwrap();
    drop(rw_tx);
    drop(pp_stop_tx);
    result_writer_handle.join().unwrap();
    progress_printer_handle.join().unwrap();

    // println!("{:#?}", recognizer.final_result().single().unwrap());
    let end_time = Instant::now();
    let secs_processed = calc_progress_in_secs(total_samples.load(Ordering::SeqCst));
    let time_elasped = end_time - start_time;
    eprintln!(
        "Processed {:.3} seconds of audio in {:.3} seconds ({:.3}x RT)",
        secs_processed,
        time_elasped.as_secs_f32(),
        secs_processed / time_elasped.as_secs_f32()
    )
}
