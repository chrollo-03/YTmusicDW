# ytmusicdw

Paste a YouTube playlist link, pick which tracks you want, download+transcode
them to Red Book CD-audio spec, and burn a real audio CD from a Rust terminal
UI — for cars (looking at you, 2011 C300) that only have a CD slot and no
Bluetooth or USB input.

```
┌ ytmusicdw ────────────────────────────────────────┐
│ [x] 01 Song A         3:42                         │
│ [x] 02 Song B         4:10                         │
│ [ ] 03 Song C (skip)  2:58                         │
│                                                     │
│ 13/14 tracks selected · 48:28 total · fits         │
│                                                     │
│ ↑/↓ move · space toggle · d download · b burn      │
└─────────────────────────────────────────────────────┘
```

## How it works

1. **Fetch** — `yt-dlp -J --flat-playlist <url>` lists every track (title,
   duration, id) without downloading anything.
2. **Pick** — toggle which tracks to include; the UI totals selected
   duration and warns if it won't fit on an 80-minute CD-R.
3. **Download** — for each selected track, `yt-dlp` pulls best audio and
   ffmpeg (via yt-dlp's postprocessor) transcodes it to 44.1kHz/16-bit
   stereo PCM WAV — exact Red Book CD-audio spec.
4. **Burn** — writes a Track-At-Once audio CD session:
   - **Windows**: uses the OS's built-in **IMAPI2** COM API
     (`scripts/burn_audio_cd.ps1`) — no extra burning software needed.
   - **Linux**: shells out to `wodim` (or `cdrecord`), which ships with most
     distros / is one `apt install wodim` away — including from an Ubuntu
     Live USB session.

## Install

Download a release from the [Releases page](../../releases) — each asset
has a build provenance attestation (no purchased code-signing cert, so
Windows SmartScreen will still warn on first run; verify authenticity with
`gh attestation verify`):

```sh
gh attestation verify ytmusicdw-windows-x86_64.exe --owner <owner>
```

Or build from source:

```sh
cargo build --release
```

### Prerequisites

| Tool                | Windows                              | Linux (e.g. Ubuntu Live) |
|---------------------|---------------------------------------|---------------------------|
| Rust                | `winget install Rustlang.Rustup`      | `curl https://sh.rustup.rs -sSf \| sh` |
| yt-dlp              | `winget install yt-dlp.yt-dlp`        | `sudo apt install yt-dlp` (or `pip install yt-dlp`) |
| ffmpeg              | `winget install Gyan.FFmpeg`          | `sudo apt install ffmpeg` |
| CD burner support   | built-in (IMAPI2)                     | `sudo apt install wodim` |

A blank CD-R and an actual optical burner (internal or USB) are required for
the burn step, obviously.

## Run

```sh
cargo run --release
```

Keys: type the playlist URL and press **Enter** to fetch · **↑/↓** move ·
**space** toggle a track · **a**/**n** select all/none · **d** download
selected · **b** burn downloaded tracks · **Esc** back · **Ctrl+C** quit.

## Notes / caveats

- Only download content you actually have the right to (your own uploads,
  Creative Commons, or otherwise permitted personal use). Respect
  creators' rights and YouTube's terms.
- The Windows IMAPI2 burn path expects a supported recorder + a blank CD-R;
  it prepares/erases media and performs one Track-At-Once write, then
  ejects.
- 80 minutes (`~700MB` at CD-audio bitrate) is the practical ceiling for a
  standard CD-R — the TUI flags playlists that won't fit.
