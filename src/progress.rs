use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Pending,
    Downloading,
    Paused,
    Verifying,
    Done,
    Skipped,
    Failed(String),
    Cancelled,
}

impl FileStatus {
    pub fn label(&self) -> String {
        match self {
            FileStatus::Pending => "queued".to_string(),
            FileStatus::Downloading => "downloading".to_string(),
            FileStatus::Paused => "paused".to_string(),
            FileStatus::Verifying => "verifying".to_string(),
            FileStatus::Done => "done".to_string(),
            FileStatus::Skipped => "skipped".to_string(),
            FileStatus::Failed(reason) => format!("failed: {reason}"),
            FileStatus::Cancelled => "cancelled".to_string(),
        }
    }
}

pub struct FileProgress {
    pub index: usize,
    pub repo: String,
    pub path: String,
    pub dest: PathBuf,
    pub total: u64,
    pub downloaded: AtomicU64,
    pub speed: AtomicU64,
    pub chunks: AtomicU64,
    pub status: Mutex<FileStatus>,
    pub history: Mutex<VecDeque<u64>>,
    pub started: Mutex<Option<Instant>>,
    pub finished: Mutex<Option<Instant>>,
}

impl FileProgress {
    pub fn new(index: usize, repo: String, path: String, dest: PathBuf, total: u64) -> Self {
        Self {
            index,
            repo,
            path,
            dest,
            total,
            downloaded: AtomicU64::new(0),
            speed: AtomicU64::new(0),
            chunks: AtomicU64::new(1),
            status: Mutex::new(FileStatus::Pending),
            history: Mutex::new(VecDeque::with_capacity(120)),
            started: Mutex::new(None),
            finished: Mutex::new(None),
        }
    }

    pub fn add(&self, bytes: u64) {
        self.downloaded.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn set_downloaded(&self, bytes: u64) {
        self.downloaded.store(bytes, Ordering::Relaxed);
    }

    pub fn progress(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn chunks(&self) -> u64 {
        self.chunks.load(Ordering::Relaxed)
    }

    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.progress() as f64 / self.total as f64).clamp(0.0, 1.0)
    }

    pub fn percent(&self) -> u16 {
        (self.fraction() * 100.0).round() as u16
    }

    pub fn set_status(&self, status: FileStatus) {
        *self.status.lock().unwrap() = status;
    }

    pub fn status(&self) -> FileStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn mark_started(&self) {
        let mut started = self.started.lock().unwrap();
        if started.is_none() {
            *started = Some(Instant::now());
        }
    }

    pub fn mark_finished(&self) {
        *self.finished.lock().unwrap() = Some(Instant::now());
    }

    pub fn elapsed(&self) -> Option<Duration> {
        self.started.lock().unwrap().map(|time| time.elapsed())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Downloading,
    Paused,
    Done,
    Failed(String),
    Cancelled,
}

pub struct ProgressHub {
    pub files: Vec<Arc<FileProgress>>,
    pub total_bytes: u64,
    pub speed: AtomicU64,
    pub phase: Mutex<Phase>,
    pub message: Mutex<String>,
    pub paused: AtomicBool,
    pub notify: tokio::sync::Notify,
    pub started: Instant,
}

impl ProgressHub {
    pub fn new(files: Vec<Arc<FileProgress>>) -> Self {
        let total_bytes = files.iter().map(|f| f.total).sum();
        Self {
            files,
            total_bytes,
            speed: AtomicU64::new(0),
            phase: Mutex::new(Phase::Preparing),
            message: Mutex::new(String::new()),
            paused: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
            started: Instant::now(),
        }
    }

    pub fn set_phase(&self, phase: Phase) {
        *self.phase.lock().unwrap() = phase;
    }

    pub fn phase(&self) -> Phase {
        self.phase.lock().unwrap().clone()
    }

    pub fn message(&self) -> String {
        self.message.lock().unwrap().clone()
    }

    pub fn set_message(&self, message: impl Into<String>) {
        *self.message.lock().unwrap() = message.into();
    }

    pub fn downloaded(&self) -> u64 {
        self.files.iter().map(|f| f.progress()).sum()
    }

    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.downloaded() as f64 / self.total_bytes as f64).clamp(0.0, 1.0)
    }

    pub fn percent(&self) -> u16 {
        (self.fraction() * 100.0).round() as u16
    }

    pub fn speed(&self) -> u64 {
        self.speed.load(Ordering::Relaxed)
    }

    pub fn eta(&self) -> String {
        let remaining = self.total_bytes.saturating_sub(self.downloaded());
        crate::util::format_eta(remaining, self.speed() as f64)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn toggle_pause(&self) {
        let now = !self.paused.load(Ordering::Relaxed);
        self.paused.store(now, Ordering::Relaxed);
        if matches!(self.phase(), Phase::Downloading | Phase::Paused) {
            self.set_phase(if now {
                Phase::Paused
            } else {
                Phase::Downloading
            });
        }
        self.notify.notify_waiters();
    }

    pub fn active_count(&self) -> usize {
        self.files
            .iter()
            .filter(|f| matches!(f.status(), FileStatus::Downloading | FileStatus::Verifying))
            .count()
    }

    pub fn done_count(&self) -> usize {
        self.files
            .iter()
            .filter(|f| matches!(f.status(), FileStatus::Done | FileStatus::Skipped))
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.files
            .iter()
            .filter(|f| matches!(f.status(), FileStatus::Failed(_)))
            .count()
    }

    /// Block while paused, returning `false` if cancellation happened first.
    pub async fn wait_unpaused(&self, cancel: &CancellationToken) -> bool {
        while self.is_paused() {
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = cancel.cancelled() => return false,
            }
        }
        !cancel.is_cancelled()
    }
}

/// Periodically recompute per-file and global speeds from downloaded counters.
pub fn spawn_sampler(hub: Arc<ProgressHub>, cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut previous: Vec<u64> = vec![0; hub.files.len()];
        let mut previous_total = 0u64;
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {
                    let mut total = 0u64;
                    for (index, file) in hub.files.iter().enumerate() {
                        let current = file.progress();
                        let delta = current.saturating_sub(previous[index]);
                        previous[index] = current;
                        let speed = delta.saturating_mul(2);
                        file.speed.store(speed, Ordering::Relaxed);

                        if hub.is_paused() {
                            if file.status() == FileStatus::Downloading {
                                file.set_status(FileStatus::Paused);
                            }
                        } else if file.status() == FileStatus::Paused {
                            file.set_status(FileStatus::Downloading);
                        }

                        if matches!(file.status(), FileStatus::Downloading | FileStatus::Verifying) {
                            let mut history = file.history.lock().unwrap();
                            if history.len() >= 120 {
                                history.pop_front();
                            }
                            history.push_back(speed);
                        }
                        total += current;
                    }
                    let global_delta = total.saturating_sub(previous_total);
                    previous_total = total;
                    hub.speed.store(global_delta.saturating_mul(2), Ordering::Relaxed);
                }
            }
        }
    })
}
