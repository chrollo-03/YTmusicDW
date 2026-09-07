use crate::burn::BurnTrack;
use crate::ytdlp::{Track, TrackTags};
use crate::{burn, convert, ytdlp};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    UrlInput,
    TrackList,
    Working,
    Done,
}

#[derive(Clone)]
pub enum TrackStatus {
    Pending,
    Downloading,
    Downloaded(PathBuf),
    Failed(String),
}

pub struct TrackItem {
    pub track: Track,
    pub selected: bool,
    pub status: TrackStatus,
}

pub enum WorkerMsg {
    Log(String),
    FetchDone(Result<Vec<Track>, String>),
    TrackDownloaded(usize, PathBuf, u64, TrackTags),
    TrackFailed(usize, String),
    DownloadBatchDone,
    /// Burn thread finished disc `n` (1-based) of `total` and is waiting for
    /// the user to swap in a fresh blank before continuing.
    AwaitDiscSwap(usize, usize),
    BurnDone(Result<(), String>),
}

/// Common blank CD-R ratings, in minutes. 'c' cycles through these in the UI
/// since 74-minute discs are just as common as 80-minute ones and the two
/// aren't interchangeable — burning past a disc's actual rated capacity
/// either fails outright or relies on unsupported overburning.
pub const CAPACITY_PRESETS_MIN: [u64; 3] = [74, 80, 90];

/// Standard Red Book minimum pause between audio tracks. Counted against
/// each disc's budget so packing doesn't quietly assume zero-gap tracks.
const TRACK_GAP_SECS: u64 = 2;

/// Headroom below a disc's nominal rated capacity: real blanks vary slightly
/// from their nominal rating, and lead-in/lead-out eats into the writable
/// area. Staying under this margin is what "100% burnable" actually means —
/// packing right up to the literal 80:00 boundary is how you get a disc that
/// fails during finalization.
const SAFETY_BUFFER_SECS: u64 = 30;

pub struct App {
    pub screen: Screen,
    pub url_input: String,
    pub tracks: Vec<TrackItem>,
    pub cursor: usize,
    pub log: Vec<String>,
    pub out_dir: PathBuf,
    pub should_quit: bool,
    pub busy: bool,
    pub error: Option<String>,
    /// Per-disc nominal capacity, in seconds (before gap/safety deductions).
    pub disc_capacity_secs: u64,
    /// Hard cap on how many discs the current selection is allowed to need.
    /// None = auto (as many discs as it takes).
    pub target_disc_count: Option<usize>,
    /// Set while the burn thread has finished one disc and is blocked
    /// waiting for the user to insert the next blank. (next_disc, total_discs)
    pub awaiting_swap: Option<(usize, usize)>,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
    continue_tx: Option<Sender<()>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// Downloads live in a `downloads/` folder next to the running exe, not
/// %TEMP% — the exe is often run from wherever it was copied to (e.g. an
/// external drive, or straight off a USB stick moved between machines), and
/// files quietly vanishing into a temp folder on a *different* machine than
/// the one you're burning on is the opposite of useful. Falls back to a
/// temp dir only if the exe's own location isn't writable/discoverable.
fn default_out_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("downloads")))
        .unwrap_or_else(|| std::env::temp_dir().join("ytmusicdw-downloads"))
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let out_dir = default_out_dir();
        Self {
            screen: Screen::UrlInput,
            url_input: String::new(),
            tracks: Vec::new(),
            cursor: 0,
            log: Vec::new(),
            out_dir,
            should_quit: false,
            busy: false,
            error: None,
            disc_capacity_secs: burn::MAX_CD_SECONDS,
            target_disc_count: None,
            awaiting_swap: None,
            tx,
            rx,
            continue_tx: None,
        }
    }

    pub fn push_log(&mut self, s: impl Into<String>) {
        self.log.push(s.into());
        if self.log.len() > 500 {
            self.log.remove(0);
        }
    }

    pub fn total_selected_seconds(&self) -> u64 {
        self.tracks
            .iter()
            .filter(|t| t.selected)
            .filter_map(|t| t.track.duration)
            .sum()
    }

    pub fn cycle_disc_capacity(&mut self) {
        let cur_min = self.disc_capacity_secs / 60;
        let idx = CAPACITY_PRESETS_MIN.iter().position(|&m| m == cur_min);
        let next = match idx {
            Some(i) => CAPACITY_PRESETS_MIN[(i + 1) % CAPACITY_PRESETS_MIN.len()],
            None => CAPACITY_PRESETS_MIN[0],
        };
        self.disc_capacity_secs = next * 60;
    }

    /// What's actually usable per disc after inter-track gaps and the safety
    /// buffer — see `SAFETY_BUFFER_SECS`.
    pub fn effective_capacity_secs(&self) -> u64 {
        self.disc_capacity_secs.saturating_sub(SAFETY_BUFFER_SECS)
    }

    pub fn inc_target_discs(&mut self) {
        let base = self.target_disc_count.unwrap_or_else(|| self.disc_groups_indices().len().max(1));
        self.target_disc_count = Some(base + 1);
    }

    pub fn dec_target_discs(&mut self) {
        let base = self.target_disc_count.unwrap_or_else(|| self.disc_groups_indices().len().max(1));
        self.target_disc_count = Some(base.saturating_sub(1).max(1));
    }

    pub fn reset_target_discs(&mut self) {
        self.target_disc_count = None;
    }

    /// Greedily packs *selected* tracks (in original playlist order) into
    /// discs of `effective_capacity_secs()` each, counting the mandatory
    /// inter-track gap against every track. Returns groups of indices into
    /// `self.tracks`. A single track longer than one disc's capacity gets
    /// its own (oversized) group rather than being dropped.
    pub fn disc_groups_indices(&self) -> Vec<Vec<usize>> {
        let cap = self.effective_capacity_secs();
        let mut groups: Vec<Vec<usize>> = vec![Vec::new()];
        let mut cur_secs = 0u64;
        for (i, t) in self.tracks.iter().enumerate() {
            if !t.selected {
                continue;
            }
            let dur = t.track.duration.unwrap_or(0) + TRACK_GAP_SECS;
            if cur_secs > 0 && cur_secs + dur > cap {
                groups.push(Vec::new());
                cur_secs = 0;
            }
            groups.last_mut().unwrap().push(i);
            cur_secs += dur;
        }
        if groups.last().is_some_and(|g| g.is_empty()) {
            groups.pop();
        }
        groups
    }

    /// If a target disc count is set and the current selection needs more
    /// discs than that, returns (discs_needed, seconds_over_budget) so the
    /// UI can tell the user exactly how much to trim.
    pub fn over_target_budget(&self) -> Option<(usize, u64)> {
        let target = self.target_disc_count?;
        let needed = self.disc_groups_indices().len().max(1);
        if needed <= target {
            return None;
        }
        let budget = target as u64 * self.effective_capacity_secs();
        let total_with_gaps: u64 = self
            .tracks
            .iter()
            .filter(|t| t.selected)
            .map(|t| t.track.duration.unwrap_or(0) + TRACK_GAP_SECS)
            .sum();
        Some((needed, total_with_gaps.saturating_sub(budget)))
    }

    /// Only tracks that downloaded successfully, grouped per disc in the
    /// same order `disc_groups_indices` would burn them, carrying metadata
    /// along for CD-Text / tracklist generation.
    pub fn downloaded_disc_groups(&self) -> Vec<Vec<BurnTrack>> {
        self.disc_groups_indices()
            .into_iter()
            .filter_map(|idxs| {
                let paths: Vec<BurnTrack> = idxs
                    .into_iter()
                    .filter_map(|i| {
                        let item = &self.tracks[i];
                        match &item.status {
                            TrackStatus::Downloaded(p) => Some(BurnTrack {
                                path: p.clone(),
                                artist: item.track.artist_label().to_string(),
                                title: item.track.title.clone(),
                                album: item.track.album.clone(),
                            }),
                            _ => None,
                        }
                    })
                    .collect();
                if paths.is_empty() { None } else { Some(paths) }
            })
            .collect()
    }

    /// Non-blocking: drains any messages the worker thread sent since last tick.
    pub fn poll_worker(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                WorkerMsg::Log(s) => self.push_log(s),
                WorkerMsg::FetchDone(res) => {
                    self.busy = false;
                    match res {
                        Ok(tracks) => {
                            self.tracks = tracks
                                .into_iter()
                                .map(|t| TrackItem { track: t, selected: true, status: TrackStatus::Pending })
                                .collect();
                            self.cursor = 0;
                            self.screen = Screen::TrackList;
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                WorkerMsg::TrackDownloaded(idx, path, secs, tags) => {
                    if let Some(t) = self.tracks.get_mut(idx) {
                        t.status = TrackStatus::Downloaded(path);
                        if t.track.duration.is_none() {
                            t.track.duration = Some(secs);
                        }
                        if let Some(a) = tags.artist {
                            t.track.artist = Some(a);
                        }
                        if let Some(title) = tags.title {
                            t.track.title = title;
                        }
                        if let Some(album) = tags.album {
                            t.track.album = Some(album);
                        }
                    }
                }
                WorkerMsg::TrackFailed(idx, err) => {
                    if let Some(t) = self.tracks.get_mut(idx) {
                        t.status = TrackStatus::Failed(err.clone());
                    }
                    self.push_log(format!("  failed: {err}"));
                }
                WorkerMsg::DownloadBatchDone => {
                    self.busy = false;
                    self.push_log("All downloads finished.");
                }
                WorkerMsg::AwaitDiscSwap(disc_num, total) => {
                    self.awaiting_swap = Some((disc_num, total));
                    let just_finished = disc_num - 1;
                    self.push_log(format!(
                        "Disc {just_finished}/{total} done. Insert the next blank CD-R and press Enter."
                    ));
                }
                WorkerMsg::BurnDone(res) => {
                    self.busy = false;
                    self.awaiting_swap = None;
                    match res {
                        Ok(()) => {
                            self.push_log("Burn finished successfully.");
                            self.screen = Screen::Done;
                        }
                        Err(e) => {
                            self.error = Some(e.clone());
                            self.push_log(format!("Burn failed: {e}"));
                        }
                    }
                }
            }
        }
    }

    pub fn start_fetch(&mut self) {
        if self.url_input.trim().is_empty() || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.log.clear();
        self.push_log(format!("Fetching playlist metadata for: {}", self.url_input.trim()));
        let url = self.url_input.trim().to_string();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let res = ytdlp::list_playlist(&url).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMsg::FetchDone(res));
        });
    }

    /// Shared guard for the download/burn actions: refuses to proceed while
    /// the selection needs more discs than the configured target, and
    /// explains exactly how much to trim instead of silently overflowing.
    fn enforce_disc_budget(&mut self) -> bool {
        if let Some((needed, over)) = self.over_target_budget() {
            self.error = Some(format!(
                "selection needs {needed} discs but target is {} — remove about {}:{:02} of tracks to fit",
                self.target_disc_count.unwrap(),
                over / 60,
                over % 60
            ));
            return false;
        }
        true
    }

    pub fn start_download_selected(&mut self) {
        if self.busy {
            return;
        }
        if !self.enforce_disc_budget() {
            return;
        }
        let jobs: Vec<(usize, Track)> = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.selected)
            .map(|(i, t)| (i, t.track.clone()))
            .collect();
        if jobs.is_empty() {
            self.error = Some("no tracks selected".to_string());
            return;
        }
        self.busy = true;
        self.screen = Screen::Working;
        self.log.clear();
        self.push_log(format!("Downloading {} track(s) to {}", jobs.len(), self.out_dir.display()));

        for (i, t) in self.tracks.iter_mut().enumerate() {
            if jobs.iter().any(|(idx, _)| *idx == i) {
                t.status = TrackStatus::Downloading;
            }
        }

        let out_dir = self.out_dir.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            for (idx, track) in jobs {
                let _ = tx.send(WorkerMsg::Log(format!("[{idx}] downloading: {}", track.title)));
                match ytdlp::download_track(&track, &out_dir) {
                    Ok((path, tags)) => {
                        if let Err(e) = convert::verify_cd_spec(&path) {
                            let _ = tx.send(WorkerMsg::TrackFailed(idx, e.to_string()));
                            continue;
                        }
                        let secs = convert::wav_duration_seconds(&path).unwrap_or(0);
                        let _ = tx.send(WorkerMsg::TrackDownloaded(idx, path, secs, tags));
                    }
                    Err(e) => {
                        let _ = tx.send(WorkerMsg::TrackFailed(idx, e.to_string()));
                    }
                }
            }
            let _ = tx.send(WorkerMsg::DownloadBatchDone);
        });
    }

    pub fn start_burn(&mut self) {
        if self.busy {
            return;
        }
        if !self.enforce_disc_budget() {
            return;
        }
        let disc_groups = self.downloaded_disc_groups();
        if disc_groups.is_empty() {
            self.error = Some("no downloaded tracks to burn yet".to_string());
            return;
        }
        if !burn::burner_available() {
            self.error = Some("no CD/DVD burner detected — plug one in and insert a blank CD-R".to_string());
            return;
        }
        self.busy = true;
        self.screen = Screen::Working;
        let total_discs = disc_groups.len();
        self.push_log(format!(
            "Starting burn: {total_discs} disc(s), {} track(s) total{}",
            disc_groups.iter().map(Vec::len).sum::<usize>(),
            if burn::supports_cd_text() { " (with CD-Text)" } else { " (no CD-Text on Windows — see tracklist.txt)" }
        ));

        let (continue_tx, continue_rx) = channel::<()>();
        self.continue_tx = Some(continue_tx);

        let tx = self.tx.clone();
        thread::spawn(move || {
            let tx_progress = tx.clone();
            let progress = move |msg: &str| {
                let _ = tx_progress.send(WorkerMsg::Log(msg.to_string()));
            };
            let tx_swap = tx.clone();
            let mut await_swap = move |disc_num: usize, total: usize| {
                let _ = tx_swap.send(WorkerMsg::AwaitDiscSwap(disc_num, total));
                let _ = continue_rx.recv(); // blocks until confirm_disc_swap() fires
            };
            let res = burn::burn_audio_cds(&disc_groups, &progress, &mut await_swap).map_err(|e| e.to_string());
            let _ = tx.send(WorkerMsg::BurnDone(res));
        });
    }

    /// Called when the user presses Enter after swapping in the next blank disc.
    pub fn confirm_disc_swap(&mut self) {
        if let Some(tx) = self.continue_tx.take() {
            let _ = tx.send(());
        }
        self.awaiting_swap = None;
    }
}
