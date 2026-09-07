//! Burns sets of CD-spec WAV files as Red Book audio CDs, so each disc plays
//! in any car stereo (no ripping/USB/Bluetooth needed on the player's end).
//!
//! Windows uses the OS's built-in IMAPI2 Track-At-Once API (no extra install).
//! IMAPI2 has no practical public API for writing CD-Text, so the Windows
//! path burns audio-only and instead writes a plain-text tracklist next to
//! the disc's downloads.
//!
//! Linux uses `cdrdao`, which *does* support authoring real CD-Text (title +
//! artist per track, readable by head units that support it) via a
//! generated `.toc` file. Falls back to a clear error telling the user to
//! `apt install cdrdao` if it's missing.
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

/// One track ready to burn: its CD-spec WAV file plus whatever metadata we
/// managed to recover (falls back to "Unknown Artist" / the video title).
#[derive(Clone)]
pub struct BurnTrack {
    pub path: PathBuf,
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
}

/// Burns `discs` (each an ordered list of tracks) as that many separate
/// audio CDs. `progress` gets human-readable status lines. `await_swap`
/// (next_disc_number, total_discs) is called before every disc after the
/// first — it should block until the caller has confirmed a fresh blank is
/// in the drive.
pub fn burn_audio_cds(
    discs: &[Vec<BurnTrack>],
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
        burn_one_disc(tracks, i + 1, total, progress)?;
    }
    Ok(())
}

fn burn_one_disc(tracks: &[BurnTrack], disc_num: usize, total_discs: usize, progress: &dyn Fn(&str)) -> Result<()> {
    for t in tracks {
        anyhow::ensure!(t.path.exists(), "missing track file: {}", t.path.display());
    }

    #[cfg(windows)]
    {
        windows::burn(tracks, disc_num, total_discs, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = total_discs;
        linux::burn(tracks, disc_num, progress)
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

/// True on the Linux/cdrdao path, false on Windows (IMAPI2 has no practical
/// CD-Text API) — used by the UI to explain up front that track/artist
/// names won't show on the car display when burning from Windows.
pub fn supports_cd_text() -> bool {
    cfg!(not(windows))
}
