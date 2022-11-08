use std::{env, time::Instant};

use arrayvec::ArrayVec;
use itertools::Itertools;
use rodio::{Decoder, Source};
use vosk::{Model, Recognizer};

const SAMPLES_BUFFER_SIZE: usize = 8 * 1024; // 8 kb

// TODO: find a way to parallelize the workload

// TODO: create function that returns iterator of recognition results

fn main() {
    let mut args = env::args();
    args.next();

    let model_path = args.next().expect("A model path was not provided");
    let audio_file_path = args
        .next()
        .expect("A path for the audio file to be read was not provided");

    // FIXME: need to convert to mono
    let file = std::fs::File::open(audio_file_path).expect("Could not open file");
    // let buf_reader = std::io::BufReader::with_capacity(SAMPLES_BUFFER_SIZE * 8, file);
    let source = Decoder::new(file).expect("Could not create the audio reader");
    let num_channels = source.channels();
    let sample_rate = source.sample_rate();

    let model = Model::new(model_path).expect("Could not create the model");
    let mut recognizer =
        Recognizer::new(&model, sample_rate as f32).expect("Could not create the recognizer");

    recognizer.set_max_alternatives(0);
    recognizer.set_words(true);
    recognizer.set_partial_words(false);

    let start_time = Instant::now();

    let mut buffer: ArrayVec<i16, SAMPLES_BUFFER_SIZE> = ArrayVec::new();
    // TODO: is there a faster way to keep reading the samples into a buffer?

    let mut total_samples = 0u64;
    for chunk in source.chunks(SAMPLES_BUFFER_SIZE).into_iter() {
        for sample in chunk {
            // let sample = sample_result.expect("Error reading sample");
            buffer.push(sample);
            total_samples += 1;
        }

        if let vosk::DecodingState::Finalized = recognizer.accept_waveform(&buffer) {
            println!("{:#?}", recognizer.result());
        }

        buffer.clear();
    }

    println!("{:#?}", recognizer.final_result().single().unwrap());
    let end_time = Instant::now();
    let secs_processed = total_samples as f32 / sample_rate as f32 / num_channels as f32;
    let time_elasped = end_time - start_time;
    eprintln!(
        "Processed {:.3} seconds of audio in {:.3} seconds ({:.3}x RT)",
        secs_processed,
        time_elasped.as_secs_f32(),
        secs_processed / time_elasped.as_secs_f32()
    )
}
