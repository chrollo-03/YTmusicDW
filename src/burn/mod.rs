//! Burns a set of CD-spec WAV files as a Red Book audio CD, so the disc plays
//! in any car stereo (no ripping/USB/Bluetooth needed on the player's end).
//!
//! Windows uses the OS's built-in IMAPI2 Track-At-Once API (no extra install).
//! Linux uses `wodim` (or falls back to `cdrecord`), which ships with most
//! distros / is one `apt install wodim` away, including on an Ubuntu Live USB.

use anyhow::Result;
use std::path::Path;

#[cfg(windows)]
mod windows;
#[cfg(not(windows))]
mod linux;

/// Burns `tracks` (in order) as one audio CD. `progress` is called with
/// human-readable status lines as the burn advances.
pub fn burn_audio_cd(tracks: &[std::path::PathBuf], progress: impl Fn(&str)) -> Result<()> {
    for t in tracks {
        anyhow::ensure!(t.exists(), "missing track file: {}", t.display());
    }

    #[cfg(windows)]
    {
        windows::burn(tracks, progress)
    }
    #[cfg(not(windows))]
    {
        linux::burn(tracks, progress)
    }
}

/// Checks (best-effort) whether a burner is reachable at all, so the UI can
/// warn early instead of failing deep into the burn.
pub fn burner_available() -> bool {
    #[cfg(windows)]
    {
        windows::recorder_present()
    }
    #[cfg(not(windows))]
    {
        linux::recorder_present()
    }
}

pub const MAX_CD_SECONDS: u64 = 80 * 60; // standard 80-minute CD-R

pub fn fits_on_one_cd(total_seconds: u64) -> bool {
    total_seconds <= MAX_CD_SECONDS
}

#[allow(dead_code)]
pub fn wav_path_ok(p: &Path) -> bool {
    p.extension().map(|e| e == "wav").unwrap_or(false)
}
