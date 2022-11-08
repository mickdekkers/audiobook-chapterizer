use std::{env, time::Instant};

use arrayvec::ArrayVec;
use hound::WavReader;
use itertools::Itertools;
use vosk::{Model, Recognizer};

const SAMPLES_BUFFER_SIZE: usize = 8 * 1024; // 8 kb

// TODO: find a way to parallelize the workload

// TODO: create function that returns iterator of recognition results

fn main() {
    let mut args = env::args();
    args.next();

    let model_path = args.next().expect("A model path was not provided");
    let wav_path = args
        .next()
        .expect("A path for the wav file to be read was not provided");

    let file = std::fs::File::open(wav_path).expect("Could not open file");
    let buf_reader = std::io::BufReader::with_capacity(SAMPLES_BUFFER_SIZE * 8, file);
    let mut wav_reader = WavReader::new(buf_reader).expect("Could not create the WAV reader");

    let model = Model::new(model_path).expect("Could not create the model");
    let mut recognizer = Recognizer::new(&model, wav_reader.spec().sample_rate as f32)
        .expect("Could not create the recognizer");

    recognizer.set_max_alternatives(0);
    recognizer.set_words(true);
    recognizer.set_partial_words(false);

    let start_time = Instant::now();

    let mut buffer: ArrayVec<i16, SAMPLES_BUFFER_SIZE> = ArrayVec::new();
    // TODO: is there a faster way to keep reading the samples into a buffer?
    for chunk in wav_reader.samples().chunks(SAMPLES_BUFFER_SIZE).into_iter() {
        for sample_result in chunk {
            let sample = sample_result.expect("Error reading sample");
            buffer.push(sample);
        }

        if let vosk::DecodingState::Finalized = recognizer.accept_waveform(&buffer) {
            println!("{:#?}", recognizer.result());
        }

        buffer.clear();
    }

    println!("{:#?}", recognizer.final_result().single().unwrap());
    let end_time = Instant::now();
    let secs_processed = wav_reader.duration() as f32 / wav_reader.spec().sample_rate as f32;
    let time_elasped = end_time - start_time;
    eprintln!(
        "Processed {:.3} seconds of audio in {:.3} seconds ({:.3}x RT)",
        secs_processed,
        time_elasped.as_secs_f32(),
        secs_processed / time_elasped.as_secs_f32()
    )
}
