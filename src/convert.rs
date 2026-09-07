//! Small helpers around the WAV files yt-dlp/ffmpeg produce: verifying they
//! actually landed at CD-audio spec, and reading their real duration (more
//! accurate than YouTube's reported duration for capacity math).

use anyhow::{Context, Result};
use std::path::Path;

pub fn verify_cd_spec(path: &Path) -> Result<()> {
    let reader = hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.sample_rate == 44100 && spec.channels == 2 && spec.bits_per_sample == 16,
        "{} is {}Hz/{}-bit/{}ch, expected 44100Hz/16-bit/stereo (CD spec)",
        path.display(),
        spec.sample_rate,
        spec.bits_per_sample,
        spec.channels
    );
    Ok(())
}

pub fn wav_duration_seconds(path: &Path) -> Result<u64> {
    let reader = hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    let frames = reader.duration() as u64; // samples per channel
    if spec.sample_rate == 0 {
        return Ok(0);
    }
    Ok(frames / spec.sample_rate as u64)
}
