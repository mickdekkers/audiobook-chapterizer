use std::collections::VecDeque;
use std::fs::File;
use std::time::Duration;

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::conv::FromSample;
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct AudioProvider {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_info: Track,
    queue: VecDeque<i16>,
    sample_rate: u32,
}

impl AudioProvider {
    pub fn new(src: File) -> Self {
        // Create the media source stream.
        let mss = MediaSourceStream::new(Box::new(src), Default::default());

        // Create a probe hint using the file's extension. [Optional]
        let hint = Hint::new();
        // hint.with_extension("mp3");

        // Use the default options for metadata and format readers.
        let meta_opts: MetadataOptions = Default::default();
        let fmt_opts: FormatOptions = Default::default();

        // Probe the media source.
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &meta_opts)
            .expect("unsupported format");

        // Get the instantiated format reader.
        let format = probed.format;

        // Find the first audio track with a known (decodeable) codec.
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .expect("no supported audio tracks");

        // Use the default options for the decoder.
        let dec_opts: DecoderOptions = Default::default();

        // Create a decoder for the track.
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &dec_opts)
            .expect("unsupported codec");

        Self {
            sample_rate: track
                .codec_params
                .sample_rate
                .expect("no sample rate in track metadata"),
            track_info: track.clone(),
            format,
            decoder,
            queue: VecDeque::new(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn total_duration(&self) -> Option<Duration> {
        let time_base = self.track_info.codec_params.time_base?;
        let n_frames = self.track_info.codec_params.n_frames?;
        let time = time_base.calc_time(n_frames);
        Some(Duration::from_secs_f64(time.seconds as f64 + time.frac))
    }
}

impl Iterator for AudioProvider {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<i16> {
        if !self.queue.is_empty() {
            return Some(self.queue.pop_front().unwrap());
        }

        // The decode loop.
        let decoded = loop {
            // Get the next packet from the media format.
            let packet = match self.format.next_packet() {
                Ok(packet) => Some(packet),
                Err(Error::ResetRequired) => {
                    // The track list has been changed. Re-examine it and create a new set of decoders,
                    // then restart the decode loop. This is an advanced feature and it is not
                    // unreasonable to consider this "the end." As of v0.5.0, the only usage of this is
                    // for chained OGG physical streams.
                    unimplemented!();
                }
                Err(err) => {
                    // eprintln!("{:#?}", err);
                    match err {
                        // https://github.com/pdeljanov/Symphonia/issues/62#issuecomment-948251294
                        Error::IoError(err)
                            if matches!(err.kind(), std::io::ErrorKind::UnexpectedEof) =>
                        {
                            break None
                        }
                        // A unrecoverable error occured, halt decoding.
                        _ => panic!("{}", err),
                    }
                }
            };

            // If there are no more packets, we've reached the end of the stream
            let packet = match packet {
                Some(packet) => packet,
                None => return None,
            };

            // Consume any new metadata that has been read since the last packet.
            while !self.format.metadata().is_latest() {
                // Pop the old head of the metadata queue.
                self.format.metadata().pop();

                // Consume the new metadata at the head of the metadata queue.
            }

            // If the packet does not belong to the selected track, skip over it.
            if packet.track_id() != self.track_info.id {
                continue;
            }

            // Decode the packet into audio samples.
            match self.decoder.decode(&packet) {
                Ok(decoded) => break Some(decoded),
                Err(Error::IoError(_)) => {
                    // The packet failed to decode due to an IO error, skip the packet.
                    continue;
                }
                Err(Error::DecodeError(_)) => {
                    // TODO: track number of decode errors encountered and bail if > threshold
                    // The packet failed to decode due to invalid data, skip the packet.
                    continue;
                }
                Err(err) => {
                    // An unrecoverable error occured, halt decoding.
                    panic!("{}", err);
                }
            }
        };

        if let Some(decoded) = decoded {
            // Consume the decoded audio samples (see below).
            // TODO: use dithering when converting sample?
            // TODO: instead of only taking from 1 channel, mix multiple channels into mono?
            // TODO: refactor this
            let target_channel = 0usize;
            match decoded {
                AudioBufferRef::F32(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::U8(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::U16(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::U24(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::U32(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::S8(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::S16(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::S24(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::S32(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
                AudioBufferRef::F64(buf) => {
                    for &sample in buf.chan(target_channel) {
                        self.queue.push_back(i16::from_sample(sample));
                    }
                }
            }
        }

        self.queue.pop_front()
    }
}
