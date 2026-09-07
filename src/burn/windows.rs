//! Windows backend: strips WAV headers to raw 44.1kHz/16-bit/stereo PCM
//! (what IMAPI2's Track-At-Once API expects) and hands them to
//! `scripts/burn_audio_cd.ps1`, which drives IMAPI2 via COM.
//!
//! IMAPI2 has no practical public API for writing CD-Text, so this path
//! burns audio-only and instead writes a plain-text tracklist next to the
//! source WAVs — print it and drop it in the CD case.

use super::BurnTrack;
use anyhow::{bail, Context, Result};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn recorder_present() -> bool {
    run_probe_script().unwrap_or(false)
}

fn run_probe_script() -> Result<bool> {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(New-Object -ComObject IMAPI2.MsftDiscMaster2).Count",
        ])
        .output()?;
    let n: i64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
    Ok(n > 0)
}

pub fn burn(tracks: &[BurnTrack], disc_num: usize, total_discs: usize, progress: &dyn Fn(&str)) -> Result<()> {
    if !recorder_present() {
        bail!("no CD/DVD recorder detected by Windows (IMAPI2). Plug in the burner and insert a blank CD-R.");
    }

    if let Some(parent) = tracks.first().and_then(|t| t.path.parent()) {
        if let Err(e) = write_tracklist(parent, tracks, disc_num) {
            progress(&format!("(couldn't write tracklist.txt: {e})"));
        } else {
            progress(&format!(
                "Note: Windows burning has no CD-Text support — wrote disc{disc_num}_tracklist.txt next to your downloads to print instead."
            ));
        }
    }

    progress("Converting tracks to raw CD-audio PCM...");
    let tmp_dir = env::temp_dir().join(format!("ytmusicdw-burn-{}-{disc_num}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir)?;

    let mut raw_paths = Vec::with_capacity(tracks.len());
    for (i, t) in tracks.iter().enumerate() {
        let raw_path = tmp_dir.join(format!("track_{i:03}.raw"));
        strip_wav_to_raw(&t.path, &raw_path)
            .with_context(|| format!("converting {} for burning", t.path.display()))?;
        raw_paths.push(raw_path);
    }

    let script = locate_burn_script()?;
    progress(&format!("Handing disc {disc_num}/{total_discs} to Windows IMAPI2 (this can take a few minutes)..."));

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-TrackFiles");
    let joined = raw_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(",");
    cmd.arg(joined);

    // .output(), not .status(): PowerShell's own console writes would
    // otherwise collide with the TUI's alternate screen.
    let out = cmd.output().context("failed to launch PowerShell burn script")?;
    let _ = std::fs::remove_dir_all(&tmp_dir);

    if !out.status.success() {
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        bail!("burn script exited with an error:\n{}", combined.trim());
    }
    progress("Burn complete. Ejecting disc.");
    Ok(())
}

fn write_tracklist(dir: &Path, tracks: &[BurnTrack], disc_num: usize) -> Result<()> {
    let mut out = format!("ytmusicdw — disc {disc_num} tracklist\n{}\n\n", "-".repeat(32));
    for (i, t) in tracks.iter().enumerate() {
        match &t.album {
            Some(album) => out.push_str(&format!("{:>2}. {} — {} [{album}]\n", i + 1, t.title, t.artist)),
            None => out.push_str(&format!("{:>2}. {} — {}\n", i + 1, t.title, t.artist)),
        }
    }
    std::fs::write(dir.join(format!("disc{disc_num}_tracklist.txt")), out)?;
    Ok(())
}

/// Reads a WAV file (must be 44100Hz/16-bit/2ch — enforced upstream during
/// conversion) and writes just its PCM sample bytes, no RIFF header.
fn strip_wav_to_raw(wav_path: &Path, raw_out: &Path) -> Result<()> {
    let mut reader = hound::WavReader::open(wav_path)
        .with_context(|| format!("opening {}", wav_path.display()))?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.sample_rate == 44100 && spec.channels == 2 && spec.bits_per_sample == 16,
        "{} is not CD-audio spec (44100Hz/16-bit/stereo); got {}Hz/{}-bit/{}ch",
        wav_path.display(),
        spec.sample_rate,
        spec.bits_per_sample,
        spec.channels
    );

    let mut out = std::fs::File::create(raw_out)?;
    use std::io::Write;
    let mut buf = Vec::with_capacity(1 << 16);
    for sample in reader.samples::<i16>() {
        buf.extend_from_slice(&sample?.to_le_bytes());
        if buf.len() >= (1 << 16) {
            out.write_all(&buf)?;
            buf.clear();
        }
    }
    if !buf.is_empty() {
        out.write_all(&buf)?;
    }
    Ok(())
}

/// The .ps1 lives in `scripts/` next to the repo/install. We look next to the
/// running exe first (for installed/release builds), then fall back to the
/// cargo project layout for `cargo run`.
fn locate_burn_script() -> Result<PathBuf> {
    let candidates = [
        env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("scripts/burn_audio_cd.ps1"))),
        Some(PathBuf::from("scripts/burn_audio_cd.ps1")),
        env::current_dir().ok().map(|d| d.join("scripts/burn_audio_cd.ps1")),
    ];
    for c in candidates.into_iter().flatten() {
        if c.exists() {
            return Ok(c);
        }
    }
    bail!("could not find scripts/burn_audio_cd.ps1 (expected next to the executable or in the project root)")
}
