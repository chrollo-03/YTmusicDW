//! Burns sets of CD-spec WAV files as Red Book audio CDs, so each disc plays
//! in any car stereo (no ripping/USB/Bluetooth needed on the player's end).
//!
//! Windows uses the OS's built-in IMAPI2 Track-At-Once API (no extra install).
//! Linux uses `wodim` (or falls back to `cdrecord`), which ships with most
//! distros / is one `apt install wodim` away, including on an Ubuntu Live USB.
//!
//! A playlist that doesn't fit one disc is burned as multiple discs: each
//! group in `discs` becomes one disc, burned in order. Between discs,
//! `await_swap` is called so the caller can pause and let the user swap in a
//! fresh blank before the next write starts.

use anyhow::Result;
use std::path::PathBuf;

#[cfg(windows)]
mod windows;
#[cfg(not(windows))]
mod linux;

pub const MAX_CD_SECONDS: u64 = 80 * 60; // standard 80-minute CD-R

/// Burns `discs` (each an ordered list of track WAV files) as that many
/// separate audio CDs. `progress` gets human-readable status lines.
/// `await_swap(next_disc_number, total_discs)` is called before every disc
/// after the first — it should block until the caller has confirmed a fresh
/// blank is in the drive.
pub fn burn_audio_cds(
    discs: &[Vec<PathBuf>],
    progress: &dyn Fn(&str),
    await_swap: &mut dyn FnMut(usize, usize),
) -> Result<()> {
    anyhow::ensure!(!discs.is_empty(), "nothing to burn");
    let total = discs.len();
    for (i, tracks) in discs.iter().enumerate() {
        anyhow::ensure!(!tracks.is_empty(), "disc {} has no downloaded tracks", i + 1);
        if i > 0 {
            await_swap(i + 1, total);
        }
        progress(&format!("Burning disc {} of {total} ({} track(s))...", i + 1, tracks.len()));
        burn_one_disc(tracks, progress)?;
    }
    Ok(())
}

fn burn_one_disc(tracks: &[PathBuf], progress: &dyn Fn(&str)) -> Result<()> {
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
