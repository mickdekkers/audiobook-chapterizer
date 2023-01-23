// Copied/adapted from https://github.com/theduke/ffprobe-rs, MIT License

use num_traits::ToPrimitive;
use serde_with::{serde_as, DisplayFromStr};
use std::{
    collections::HashMap,
    error, fmt, io,
    path::Path,
    process::{self, Command},
    time::Duration,
};

/// Execute ffprobe and return the extracted data.
pub fn ffprobe(path: impl AsRef<Path>) -> Result<FfProbe, FfProbeError> {
    let path = path.as_ref();

    let mut cmd = Command::new("ffprobe");

    // Default args.
    cmd.args(["-v", "quiet", "-show_chapters", "-print_format", "json"]);

    cmd.arg(path);

    let out = cmd.output().map_err(FfProbeError::Io)?;

    if !out.status.success() {
        return Err(FfProbeError::Status(out));
    }

    serde_json::from_slice::<FfProbe>(&out.stdout).map_err(FfProbeError::Deserialize)
}

#[derive(Debug)]
#[non_exhaustive]
pub enum FfProbeError {
    Io(io::Error),
    Status(process::Output),
    Deserialize(serde_json::Error),
}

impl fmt::Display for FfProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FfProbeError::Io(e) => e.fmt(f),
            FfProbeError::Status(o) => {
                write!(
                    f,
                    "ffprobe exited with status code {}: {}",
                    o.status,
                    String::from_utf8_lossy(&o.stderr)
                )
            }
            FfProbeError::Deserialize(e) => e.fmt(f),
        }
    }
}

impl error::Error for FfProbeError {}

#[derive(Default, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FfProbe {
    pub chapters: Vec<Chapter>,
}

#[serde_as]
#[derive(Default, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Chapter {
    pub id: i64,
    #[serde_as(as = "DisplayFromStr")]
    time_base: num_rational::Rational32,
    start: i64,
    end: i64,
    tags: Option<HashMap<String, String>>,
}

impl Chapter {
    pub fn title(&self) -> Option<&str> {
        self.tags
            .as_ref()
            .and_then(|tags| tags.get("title"))
            .map(|x| &**x)
    }

    pub fn description(&self) -> Option<&str> {
        self.tags
            .as_ref()
            .and_then(|tags| tags.get("description"))
            .map(|x| &**x)
    }

    pub fn start(&self) -> Duration {
        Duration::from_secs_f64(self.start as f64 * self.time_base.to_f64().unwrap())
    }

    pub fn end(&self) -> Duration {
        Duration::from_secs_f64(self.end as f64 * self.time_base.to_f64().unwrap())
    }
}
