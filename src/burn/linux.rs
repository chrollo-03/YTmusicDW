//! Linux backend: shells out to `wodim` (preferred) or `cdrecord` in audio
//! mode. Both accept standard WAV files directly and strip the header
//! themselves, so no raw-PCM conversion is needed here.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

fn burner_binary() -> Option<&'static str> {
    ["wodim", "cdrecord"]
        .into_iter()
        .find(|bin| Command::new("which").arg(bin).output().map(|o| o.status.success()).unwrap_or(false))
}

pub fn recorder_present() -> bool {
    let Some(bin) = burner_binary() else { return false };
    // `-scanbus` lists attached recorders; treat any non-empty CD/DVD line as present.
    Command::new(bin)
        .arg("-scanbus")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.contains("CD") || l.contains("DVD")))
        .unwrap_or(false)
}

pub fn burn(wav_tracks: &[PathBuf], progress: &dyn Fn(&str)) -> Result<()> {
    let bin = burner_binary().ok_or_else(|| {
        anyhow::anyhow!("neither `wodim` nor `cdrecord` found. On Ubuntu: `sudo apt install wodim`")
    })?;

    progress(&format!("Burning {} tracks with {bin} (insert a blank CD-R first)...", wav_tracks.len()));

    let mut cmd = Command::new(bin);
    cmd.args(["-v", "-pad", "-audio", "-eject"]);
    for t in wav_tracks {
        cmd.arg(t);
    }

    let status = cmd.status().with_context(|| format!("failed to launch {bin}"))?;
    if !status.success() {
        bail!("{bin} exited with an error — check that a blank CD-R is inserted and the drive isn't in use");
    }
    progress("Burn complete, disc ejected.");
    Ok(())
}
