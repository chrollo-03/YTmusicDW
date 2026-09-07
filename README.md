# ytmusicdw

Paste a YouTube playlist link, pick which tracks you want, download+transcode
them to Red Book CD-audio spec, and burn real audio CD(s) — with artist/title
metadata where the burn path supports it — from a Rust terminal UI. Built for
cars (looking at you, 2011 C300) that only have a CD slot and no Bluetooth or
USB input.

```
┌ ytmusicdw ──────────────────────────────────────────────────┐
│ [x] CD1  01 Song A — Artist One               3:42           │
│ [x] CD1  02 Song B — Artist Two               4:10           │
│ [x] CD2  03 Song C — Artist Three             2:58           │
│                                                                │
│ 13/14 tracks selected · 48:28 total · needs 2 disc(s) @ 80min │
│ target: auto · effective budget 78:34/disc (80min minus 2s    │
│ track gaps + 30s safety margin) · '[' ']' set target, '0' auto│
└────────────────────────────────────────────────────────────────┘
```

## How it works

1. **Fetch** — `yt-dlp -J --flat-playlist <url>` lists every track (title,
   duration, id, and — when the playlist page has it — uploader/channel as a
   cheap first-pass artist guess) without downloading anything.
2. **Pick** — toggle which tracks to include. Selected tracks are greedily
   packed into discs, in playlist order, shown as `CD1`/`CD2`/... tags.
3. **Download** — for each selected track, `yt-dlp` pulls best audio,
   ffmpeg (via yt-dlp's postprocessor) transcodes it to 44.1kHz/16-bit
   stereo PCM WAV — exact Red Book CD-audio spec — and `--write-info-json`
   is used to recover real `artist`/`track`/`album` tags when YouTube has
   them (reliable for YouTube Music links, hit-or-miss for plain videos;
   falls back to uploader name / video title otherwise).
4. **Burn** — one Track-At-Once session per disc, pausing between discs so
   you can swap in a fresh blank:
   - **Linux**: uses `cdrdao`, which authors a real **CD-Text** block (disc
     title/album + per-track title/performer) into the burn — the only path
     here that can put artist/title on a car display that supports it. One
     `apt install cdrdao` away, including from an Ubuntu Live USB session.
   - **Windows**: uses the OS's built-in **IMAPI2** COM API
     (`scripts/burn_audio_cd.ps1`) — no extra burning software needed, but
     IMAPI2 has no practical public API for writing CD-Text, so this path
     burns audio-only and writes a `discN_tracklist.txt` next to your
     downloads instead — print it and drop it in the CD case.

### Capacity: why "just fit it in 700MB" doesn't apply

Audio CDs aren't measured in file size the way data CDs are. Red Book audio
is raw, uncompressed 44.1kHz/16-bit/stereo PCM at a fixed rate, always — a
disc's real ceiling is however many *minutes* it's rated for (74 or 80 are
the common ones; 90-minute blanks exist but are off-spec and less reliable).
There's no compressing your way around that, and overburning past a disc's
rated capacity is unsupported by most drives/media.

So the app doesn't pack right up to the nominal boundary either — each
disc's usable budget is its rated size **minus** the mandatory 2-second
Red Book pause between every track **minus** a 30-second safety margin for
lead-in/lead-out and real-world blank variance. That's what "100% burnable"
means here: headroom, not a number that just barely theoretically fits.

### Target disc count

By default the app uses as many discs as your selection needs ("auto").
Press **`[`** / **`]`** in the track list to set a hard cap — e.g. "exactly
2 discs" — and **`0`** to go back to auto. If your selection needs more
discs than the target, the app **refuses to download/burn** and tells you
exactly how much to trim (in minutes:seconds) instead of silently spilling
onto an extra disc.

## Install

Download a release from the [Releases page](../../releases) — each asset
has a build provenance attestation (no purchased code-signing cert, so
Windows SmartScreen will still warn on first run; verify authenticity with
`gh attestation verify`):

```sh
gh attestation verify ytmusicdw-windows-x86_64.exe --owner <owner>
```

**Windows: pick the right exe for your CPU/OS bitness.** Check
Settings → System → About → "System type":

- `ytmusicdw-windows-x86_64.exe` — 64-bit Windows (the normal case; this is
  what almost every PC from the last ~15 years runs, including old laptops
  like a Toshiba with a Pentium B960 that shipped with 64-bit Windows 7/10).
- `ytmusicdw-windows-x86_32bit.exe` — only if "System type" literally says
  **32-bit operating system**. Running the 64-bit exe there fails with
  "This app can't run on your PC" — that error is purely about binary
  format, not CPU power. On a genuinely 32-bit Windows you'll also need
  32-bit builds of the two external tools:
  - yt-dlp: grab `yt-dlp_x86.exe` from the
    [yt-dlp releases page](https://github.com/yt-dlp/yt-dlp/releases),
    rename it to `yt-dlp.exe`, put it on PATH.
  - ffmpeg: grab an `x86` (not `x86_64`) build from
    [yt-dlp/FFmpeg-Builds releases](https://github.com/yt-dlp/FFmpeg-Builds/releases)
    (built specifically for yt-dlp compatibility), extract `ffmpeg.exe` onto PATH.

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
| CD burner support   | built-in (IMAPI2)                     | `sudo apt install cdrdao` |

A blank CD-R and an actual optical burner (internal or USB) are required for
the burn step, obviously.

## Run

```sh
cargo run --release
```

Keys: type the playlist URL and press **Enter** to fetch · **↑/↓** move ·
**space** toggle a track · **a**/**n** select all/none · **c** cycle disc
size (74/80/90 min) · **[**/**]** set target disc count · **0** reset to
auto · **d** download selected · **b** burn downloaded tracks (pauses
between discs — swap the blank and press **Enter**) · **Esc** back ·
**Ctrl+C** quit.

## Notes / caveats

- Only download content you actually have the right to (your own uploads,
  Creative Commons, or otherwise permitted personal use). Respect
  creators' rights and YouTube's terms.
- The Windows IMAPI2 burn path expects a supported recorder + a blank CD-R;
  it prepares/erases media and performs one Track-At-Once write per disc,
  then ejects.
- CD-Text display depends entirely on the car head unit supporting it —
  plenty of factory units, especially older/base ones, just ignore it. The
  audio itself is unaffected either way.
