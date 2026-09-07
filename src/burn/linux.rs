//! Linux backend: generates a cdrdao `.toc` file with a real CD-TEXT block
//! (disc + per-track TITLE/PERFORMER) and hands it to `cdrdao write`. This
//! is the only backend here that can put artist/title on the car display —
//! Windows' IMAPI2 has no practical public API for writing CD-Text.

use super::BurnTrack;
use anyhow::{bail, Context, Result};
use std::env;
use std::path::Path;
use std::process::Command;

pub fn recorder_present() -> bool {
    if !cdrdao_installed() {
        return false;
    }
    // `-scanbus` lists attached recorders; treat any non-empty CD/DVD line as present.
    Command::new("cdrdao")
        .arg("scanbus")
        .output()
        .map(|o| {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            combined.lines().any(|l| l.contains("CD-R") || l.contains("DVD") || l.contains("CD/DVD"))
        })
        .unwrap_or(false)
}

fn cdrdao_installed() -> bool {
    Command::new("which").arg("cdrdao").output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn burn(tracks: &[BurnTrack], disc_num: usize, progress: &dyn Fn(&str)) -> Result<()> {
    if !cdrdao_installed() {
        bail!("`cdrdao` not found. On Ubuntu: `sudo apt install cdrdao`");
    }

    let toc_path = env::temp_dir().join(format!("ytmusicdw-disc{disc_num}-{}.toc", std::process::id()));
    let toc = build_toc(tracks, disc_num);
    std::fs::write(&toc_path, toc).context("writing cdrdao .toc file")?;

    progress(&format!(
        "Burning {} track(s) with cdrdao (CD-Text included; insert a blank CD-R first)...",
        tracks.len()
    ));

    let status = Command::new("cdrdao")
        .args(["write", "--eject"])
        .arg(&toc_path)
        .status()
        .context("failed to launch cdrdao")?;

    let _ = std::fs::remove_file(&toc_path);

    if !status.success() {
        bail!("cdrdao exited with an error — check that a blank CD-R is inserted and the drive isn't in use");
    }
    progress("Burn complete, disc ejected.");
    Ok(())
}

/// Builds a cdrdao TOC with a disc-level and per-track CD_TEXT block.
/// cdrdao reads standard WAV files directly (auto-strips the header), so
/// the same CD-spec WAVs used everywhere else work as-is here.
fn build_toc(tracks: &[BurnTrack], disc_num: usize) -> String {
    // Prefer a real album name (from YouTube Music tags) as the disc title;
    // fall back to a generic label when nothing on the disc has one.
    let disc_title = tracks
        .iter()
        .find_map(|t| t.album.clone())
        .unwrap_or_else(|| format!("ytmusicdw disc {disc_num}"));

    let mut toc = String::new();
    toc.push_str("CD_DA\n\n");
    toc.push_str("CD_TEXT {\n  LANGUAGE_MAP {\n    0 : EN\n  }\n  LANGUAGE 0 {\n");
    toc.push_str(&format!("    TITLE {}\n", cdtext_str(&disc_title)));
    toc.push_str(&format!("    PERFORMER {}\n", cdtext_str("Various Artists")));
    toc.push_str("  }\n}\n\n");

    for t in tracks {
        toc.push_str("TRACK AUDIO\n");
        toc.push_str("CD_TEXT {\n  LANGUAGE 0 {\n");
        toc.push_str(&format!("    TITLE {}\n", cdtext_str(&t.title)));
        toc.push_str(&format!("    PERFORMER {}\n", cdtext_str(&t.artist)));
        toc.push_str("  }\n}\n");
        toc.push_str(&format!("FILE {} 0\n\n", toc_quote(&t.path)));
    }
    toc
}

/// CD-Text fields are traditionally short (pack-based Red Book fields);
/// truncate defensively so a very long YouTube title can't break the encoder.
fn cdtext_str(s: &str) -> String {
    let truncated: String = s.chars().take(160).collect();
    format!("\"{}\"", truncated.replace('"', "'"))
}

fn toc_quote(p: &Path) -> String {
    format!("\"{}\"", p.to_string_lossy().replace('"', "'"))
}
