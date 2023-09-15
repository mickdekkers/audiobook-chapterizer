use audiobook_chapterizer::{
    chapterize::{chapterize, ChapterizeOptions},
    extract::{extract_chapters, ExtractOptions},
};
use clap::{
    builder::{OsStringValueParser, TypedValueParser},
    ArgAction, Args, Parser,
};
use color_eyre::eyre;
use log::LevelFilter;
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

// TODO: find a way to parallelize the workload

fn verify_jsonl_ext(os: OsString) -> Result<PathBuf, &'static str> {
    let path = PathBuf::from(os);
    if path.extension() != Some(OsStr::new("jsonl")) {
        return Err("path must end in .jsonl");
    }
    Ok(path)
}

#[derive(Args, Clone, Debug)]
#[group(required = true, multiple = true)]
struct Outputs {
    // TODO: verify extension of .cue
    /// The path that the output .cue file will be written to (if any).
    #[arg(value_name = "cue_file", long = "output_cue")]
    cue_file_path: Option<PathBuf>,
    /// The path that the output ffmetadata file will be written to (if any).
    #[arg(value_name = "ffmetadata_file", long = "output_ffmetadata")]
    ffmetadata_file_path: Option<PathBuf>,
}

#[derive(Parser, Clone, Debug)]
struct Cli {
    /// Makes logging more verbose. Pass once for debug log level, twice for trace log level.
    #[arg(short, action = ArgAction::Count, global = true)]
    verbose: u8,
    /// The path to the Vosk ASR model directory to use.
    #[arg(value_name = "model_dir", long = "model", default_value = "./model")]
    model_dir_path: PathBuf,
    /// Optionally, a path to a file to write matching recognition results to. The path must end in
    /// .jsonl
    #[arg(
        value_name = "matches_file",
        long = "write_matches",
        value_parser = OsStringValueParser::new().try_map(verify_jsonl_ext)
    )]
    matches_file_path: Option<PathBuf>,
    /// The path to the audio file to chapterize.
    #[arg(value_name = "audio_file", short = 'i')]
    audio_file_path: PathBuf,
    #[command(flatten)]
    outputs: Outputs,
}

impl From<Cli> for ChapterizeOptions {
    fn from(val: Cli) -> Self {
        ChapterizeOptions {
            model_dir_path: val.model_dir_path,
            matches_file_path: val.matches_file_path,
            audio_file_path: val.audio_file_path,
            cue_file_path: val.outputs.cue_file_path,
            ffmetadata_file_path: val.outputs.ffmetadata_file_path,
        }
    }
}

impl From<Cli> for ExtractOptions {
    fn from(val: Cli) -> Self {
        ExtractOptions {
            audio_file_path: val.audio_file_path,
            cue_file_path: val.outputs.cue_file_path,
            ffmetadata_file_path: val.outputs.ffmetadata_file_path,
        }
    }
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

    // TODO: add option/subcommand to skip metadata extraction and force ASR instead
    // TODO: add force-extract flag and force-asr (or similar) flag
    let metadata_chapters_found = extract_chapters(&cli.clone().into())?;

    if !metadata_chapters_found {
        chapterize(&cli.into())?;
    }

    Ok(())
}
