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
    BurnDone(Result<(), String>),
}

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
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
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
            tx,
            rx,
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
                WorkerMsg::BurnDone(res) => {
                    self.busy = false;
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

    /// Only tracks that downloaded successfully, in original playlist order.
    pub fn downloaded_paths(&self) -> Vec<PathBuf> {
        self.tracks
            .iter()
            .filter(|t| t.selected)
            .filter_map(|t| match &t.status {
                TrackStatus::Downloaded(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn start_burn(&mut self) {
        if self.busy {
            return;
        }
        let paths = self.downloaded_paths();
        if paths.is_empty() {
            self.error = Some("no downloaded tracks to burn yet".to_string());
            return;
        }
        if !burn::burner_available() {
            self.error = Some("no CD/DVD burner detected — plug one in and insert a blank CD-R".to_string());
            return;
        }
        self.busy = true;
        self.screen = Screen::Working;
        self.push_log(format!("Starting burn of {} track(s)...", paths.len()));

        let tx = self.tx.clone();
        thread::spawn(move || {
            let tx2 = tx.clone();
            let res = burn::burn_audio_cd(&paths, move |msg| {
                let _ = tx2.send(WorkerMsg::Log(msg.to_string()));
            })
            .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMsg::BurnDone(res));
        });
    }
}
