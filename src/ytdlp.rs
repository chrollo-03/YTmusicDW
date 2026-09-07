//! Talks to the `yt-dlp` binary on PATH: lists playlist entries and
//! downloads+extracts audio for the ones the user picked.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Track {
    pub id: String,
    pub title: String,
    /// Duration in whole seconds; None if yt-dlp couldn't tell us (e.g. live streams).
    pub duration: Option<u64>,
    /// Best-effort metadata, filled in cheaply from the playlist listing and
    /// then upgraded (if better tags exist) once the track is downloaded.
    pub artist: Option<String>,
    pub album: Option<String>,
}

impl Track {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }

    pub fn duration_label(&self) -> String {
        match self.duration {
            Some(s) => format!("{}:{:02}", s / 60, s % 60),
            None => "--:--".to_string(),
        }
    }

    pub fn artist_label(&self) -> &str {
        self.artist.as_deref().unwrap_or("Unknown Artist")
    }
}

/// Real tag data recovered from yt-dlp's `--write-info-json` sidecar after a
/// download. YouTube Music links tend to carry proper `artist`/`track`/`album`
/// tags; plain YouTube videos usually don't, so callers should fall back to
/// the uploader name / video title when these are None.
#[derive(Default)]
pub struct TrackTags {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
}

#[derive(Deserialize)]
struct FlatEntry {
    id: String,
    title: Option<String>,
    duration: Option<f64>,
    uploader: Option<String>,
    channel: Option<String>,
}

#[derive(Deserialize)]
struct FlatPlaylist {
    entries: Option<Vec<FlatEntry>>,
    // A lone video (not a playlist) parses as a single object with its own id/title.
    id: Option<String>,
    title: Option<String>,
    duration: Option<f64>,
    uploader: Option<String>,
    channel: Option<String>,
}

#[derive(Deserialize, Default)]
struct InfoJsonTags {
    artist: Option<String>,
    creator: Option<String>,
    uploader: Option<String>,
    track: Option<String>,
    title: Option<String>,
    album: Option<String>,
}

/// Confirms `yt-dlp` is reachable on PATH; returns its reported version string.
pub fn check_available() -> Result<String> {
    let out = Command::new("yt-dlp")
        .arg("--version")
        .output()
        .context("failed to launch yt-dlp — is it installed and on PATH?")?;
    if !out.status.success() {
        bail!("yt-dlp --version exited with an error");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Fetches metadata for every entry in a playlist (or a single video) without downloading media.
pub fn list_playlist(url: &str) -> Result<Vec<Track>> {
    let out = Command::new("yt-dlp")
        .args([
            "-J",
            "--flat-playlist",
            "--no-warnings",
            "--ignore-errors",
            url,
        ])
        .output()
        .context("failed to launch yt-dlp")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("yt-dlp failed to read that link:\n{}", stderr.trim());
    }

    let parsed: FlatPlaylist = serde_json::from_slice(&out.stdout)
        .context("could not parse yt-dlp's JSON output")?;

    let tracks = if let Some(entries) = parsed.entries {
        entries
            .into_iter()
            .map(|e| Track {
                id: e.id,
                title: e.title.unwrap_or_else(|| "(untitled)".to_string()),
                duration: e.duration.map(|d| d.round() as u64),
                artist: e.uploader.or(e.channel),
                album: None,
            })
            .collect()
    } else if let Some(id) = parsed.id {
        vec![Track {
            id,
            title: parsed.title.unwrap_or_else(|| "(untitled)".to_string()),
            duration: parsed.duration.map(|d| d.round() as u64),
            artist: parsed.uploader.or(parsed.channel),
            album: None,
        }]
    } else {
        bail!("that link didn't resolve to any tracks");
    };

    if tracks.is_empty() {
        bail!("playlist is empty (or every entry was unavailable)");
    }

    Ok(tracks)
}

/// Downloads one track's audio and transcodes it to CD-audio spec
/// (44.1kHz, 16-bit, stereo PCM WAV) via yt-dlp's ffmpeg postprocessor.
/// Also writes an info-json sidecar so real artist/title/album tags (when
/// YouTube has them — reliable for YouTube Music links, hit-or-miss for
/// plain videos) can be pulled back into the UI and CD-Text.
pub fn download_track(track: &Track, out_dir: &Path) -> Result<(PathBuf, TrackTags)> {
    std::fs::create_dir_all(out_dir)?;
    let base = sanitize(&track.id);
    let out_template = out_dir.join(format!("{base}.%(ext)s"));

    let status = Command::new("yt-dlp")
        .args([
            "-f",
            "bestaudio/best",
            "-x",
            "--audio-format",
            "wav",
            "--audio-quality",
            "0",
            "--postprocessor-args",
            "ffmpeg:-ar 44100 -ac 2 -sample_fmt s16",
            "--write-info-json",
            "--no-warnings",
            "-o",
        ])
        .arg(&out_template)
        .arg(track.url())
        .status()
        .context("failed to launch yt-dlp for download")?;

    if !status.success() {
        bail!("yt-dlp failed to download \"{}\"", track.title);
    }

    let wav_path = out_dir.join(format!("{base}.wav"));
    if !wav_path.exists() {
        bail!(
            "expected output file missing after download: {}",
            wav_path.display()
        );
    }

    let tags = read_info_json_tags(&out_dir.join(format!("{base}.info.json")));
    Ok((wav_path, tags))
}

/// Best-effort: missing/unparsable info-json just means no upgraded tags,
/// not a failed download.
fn read_info_json_tags(path: &Path) -> TrackTags {
    let Ok(bytes) = std::fs::read(path) else { return TrackTags::default() };
    let Ok(info) = serde_json::from_slice::<InfoJsonTags>(&bytes) else { return TrackTags::default() };
    TrackTags {
        artist: info.artist.or(info.creator).or(info.uploader),
        title: info.track.or(info.title),
        album: info.album,
    }
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}
