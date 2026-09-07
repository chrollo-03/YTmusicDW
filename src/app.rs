use crate::{burn, convert, ytdlp};
use crate::ytdlp::Track;
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
    TrackDownloaded(usize, PathBuf, u64),
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
    /// Per-disc capacity, in seconds. Selected tracks are greedily packed
    /// into discs of this size, in playlist order.
    pub disc_capacity_secs: u64,
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

impl App {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let out_dir = std::env::temp_dir().join("ytmusicdw-downloads");
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

    /// Greedily packs *selected* tracks (in original playlist order) into
    /// discs of `disc_capacity_secs` each. Returns groups of indices into
    /// `self.tracks`. A single track longer than one disc's capacity gets
    /// its own (oversized) group rather than being dropped.
    pub fn disc_groups_indices(&self) -> Vec<Vec<usize>> {
        let mut groups: Vec<Vec<usize>> = vec![Vec::new()];
        let mut cur_secs = 0u64;
        for (i, t) in self.tracks.iter().enumerate() {
            if !t.selected {
                continue;
            }
            let dur = t.track.duration.unwrap_or(0);
            if cur_secs > 0 && cur_secs + dur > self.disc_capacity_secs {
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

    /// Only tracks that downloaded successfully, grouped per disc in the
    /// same order `disc_groups_indices` would burn them.
    pub fn downloaded_disc_groups(&self) -> Vec<Vec<PathBuf>> {
        self.disc_groups_indices()
            .into_iter()
            .filter_map(|idxs| {
                let paths: Vec<PathBuf> = idxs
                    .into_iter()
                    .filter_map(|i| match &self.tracks[i].status {
                        TrackStatus::Downloaded(p) => Some(p.clone()),
                        _ => None,
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
                WorkerMsg::TrackDownloaded(idx, path, secs) => {
                    if let Some(t) = self.tracks.get_mut(idx) {
                        t.status = TrackStatus::Downloaded(path);
                        if t.track.duration.is_none() {
                            t.track.duration = Some(secs);
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
                    self.push_log(format!(
                        "Disc {}/{} done. Insert the next blank CD-R and press Enter.",
                        disc_num - 1,
                        total
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

    pub fn start_download_selected(&mut self) {
        if self.busy {
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
                    Ok(path) => {
                        if let Err(e) = convert::verify_cd_spec(&path) {
                            let _ = tx.send(WorkerMsg::TrackFailed(idx, e.to_string()));
                            continue;
                        }
                        let secs = convert::wav_duration_seconds(&path).unwrap_or(0);
                        let _ = tx.send(WorkerMsg::TrackDownloaded(idx, path, secs));
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
            "Starting burn: {total_discs} disc(s), {} track(s) total",
            disc_groups.iter().map(Vec::len).sum::<usize>()
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
