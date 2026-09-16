pub mod resume;
pub mod verify;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use futures::StreamExt;
use reqwest::StatusCode;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::api::{HfClient, RemoteFile, RepoInfo};
use crate::progress::{FileProgress, FileStatus, Phase, ProgressHub};
use crate::store::{self, Manifest, ManifestFile};

use resume::{ChunkState, PartState};

const MIN_CHUNK: u64 = 4 * 1024 * 1024;
const MAX_RETRIES: u32 = 8;

pub struct DownloadPlan {
    pub repo: RepoInfo,
    pub files: Vec<RemoteFile>,
    pub output_root: PathBuf,
    pub command: String,
}

pub struct RunConfig {
    pub connections: usize,
    pub max_active: usize,
    pub verify: bool,
    pub force: bool,
}

struct Engine {
    client: Arc<HfClient>,
    cancel: CancellationToken,
    conn_sem: Arc<Semaphore>,
    hub: Arc<ProgressHub>,
    connections: usize,
    verify: bool,
    force: bool,
}

/// Run all downloads for a plan, updating the shared progress hub.
pub async fn run(
    client: Arc<HfClient>,
    plan: DownloadPlan,
    hub: Arc<ProgressHub>,
    cancel: CancellationToken,
    config: RunConfig,
) {
    hub.set_phase(Phase::Downloading);
    let file_sem = Arc::new(Semaphore::new(config.max_active.max(1)));
    let conn_sem = Arc::new(Semaphore::new(
        (config.connections.max(1) * config.max_active.max(1)).clamp(1, 256),
    ));

    let engine = Arc::new(Engine {
        client,
        cancel: cancel.clone(),
        conn_sem,
        hub: hub.clone(),
        connections: config.connections.max(1),
        verify: config.verify,
        force: config.force,
    });

    let repo_path = store::repo_dir(&plan.output_root, &plan.repo.id);
    let mut handles = Vec::new();

    for (index, remote) in plan.files.iter().enumerate() {
        let progress = hub.files[index].clone();
        let engine = engine.clone();
        let repo = plan.repo.clone();
        let remote = remote.clone();
        let dest = repo_path.join(&remote.path);
        let file_sem = file_sem.clone();

        handles.push(tokio::spawn(async move {
            let permit = match file_sem.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return,
            };
            if engine.cancel.is_cancelled() {
                progress.set_status(FileStatus::Cancelled);
                return;
            }
            progress.mark_started();
            match engine.run_file(&repo, &remote, &dest, &progress).await {
                Ok(true) => {
                    progress.set_status(FileStatus::Done);
                    progress.mark_finished();
                }
                Ok(false) => {
                    progress.set_status(FileStatus::Skipped);
                    progress.mark_finished();
                }
                Err(error) => {
                    if engine.cancel.is_cancelled() {
                        progress.set_status(FileStatus::Cancelled);
                    } else {
                        progress.set_status(FileStatus::Failed(error.to_string()));
                    }
                    progress.mark_finished();
                }
            }
            drop(permit);
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }

    if cancel.is_cancelled() {
        hub.set_phase(Phase::Cancelled);
        return;
    }

    let manifest_files: Vec<ManifestFile> = plan
        .files
        .iter()
        .zip(hub.files.iter())
        .filter(|(_, progress)| matches!(progress.status(), FileStatus::Done | FileStatus::Skipped))
        .map(|(remote, _)| ManifestFile {
            path: remote.path.clone(),
            size: remote.size,
            sha256: remote.sha256.clone(),
        })
        .collect();

    if !manifest_files.is_empty() {
        let manifest = Manifest {
            repo: plan.repo.id.clone(),
            repo_type: plan.repo.repo_type.clone(),
            revision: plan.repo.revision.clone(),
            commit: plan.repo.commit.clone(),
            endpoint: client_endpoint(&engine),
            downloaded_at: crate::util::now_iso8601(),
            command: plan.command.clone(),
            total_size: manifest_files.iter().map(|f| f.size).sum(),
            files: manifest_files,
        };
        let _ = store::write_manifest(&repo_path, &manifest);
    }

    if hub.failed_count() > 0 {
        let failed = hub.failed_count();
        hub.set_phase(Phase::Failed(format!("{failed} file(s) failed")));
    } else {
        hub.set_phase(Phase::Done);
    }
}

fn client_endpoint(engine: &Arc<Engine>) -> String {
    engine.client.endpoint().to_string()
}

impl Engine {
    async fn run_file(
        &self,
        repo: &RepoInfo,
        remote: &RemoteFile,
        dest: &Path,
        progress: &Arc<FileProgress>,
    ) -> Result<bool> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let expected_sha = remote.sha256.clone();

        let already_complete = !self.force
            && !resume::sidecar_path(dest).exists()
            && remote.size > 0
            && std::fs::metadata(dest)
                .map(|metadata| metadata.len() == remote.size)
                .unwrap_or(false);

        if already_complete {
            let verified = if self.verify {
                match &expected_sha {
                    Some(sha) => {
                        progress.set_status(FileStatus::Verifying);
                        verify::verify_sha256(dest, sha).await.unwrap_or(false)
                    }
                    None => true,
                }
            } else {
                true
            };
            if verified {
                progress.set_downloaded(remote.size);
                return Ok(false);
            }
            let _ = std::fs::remove_file(dest);
            PartState::remove(dest);
        }

        let url = self
            .client
            .resolve_url(&repo.repo_type, &repo.id, &repo.revision, &remote.path);
        let probe = self.client.probe(&url).await?;
        let size = if remote.size > 0 {
            remote.size
        } else {
            probe.size.unwrap_or(0)
        };
        let etag = probe.etag.clone();

        let use_chunks = probe.supports_ranges && size > MIN_CHUNK && self.connections > 1;
        let planned = if use_chunks {
            plan_chunks(size, self.connections)
        } else {
            vec![ChunkState {
                index: 0,
                len: size,
                done: 0,
            }]
        };
        let chunk_size = planned.first().map(|c| c.len).unwrap_or(0);

        let state = match PartState::load(dest) {
            Some(previous)
                if previous.total == size
                    && previous.chunks.len() == planned.len()
                    && etag_compatible(&previous.etag, &etag) =>
            {
                previous
            }
            _ => PartState::new(
                &remote.path,
                size,
                etag.clone(),
                expected_sha.clone(),
                chunk_size,
                planned,
            ),
        };
        let preloaded = state.downloaded();
        progress.set_downloaded(preloaded);
        progress.chunks.store(
            state.chunks.len() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        progress.set_status(FileStatus::Downloading);

        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(dest)
            .with_context(|| format!("creating {}", dest.display()))?;
        if size > 0 {
            file.set_len(size)
                .with_context(|| format!("preallocating {}", dest.display()))?;
        }
        let file = Arc::new(file);
        let state = Arc::new(Mutex::new(state));

        let saver_token = self.cancel.child_token();
        let saver_state = state.clone();
        let saver_dest = dest.to_path_buf();
        let saver = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = saver_token.cancelled() => break,
                    _ = ticker.tick() => {
                        let snapshot = saver_state.lock().unwrap().clone();
                        let _ = snapshot.save(&saver_dest);
                    }
                }
            }
            let snapshot = saver_state.lock().unwrap().clone();
            let _ = snapshot.save(&saver_dest);
        });

        let chunk_count = state.lock().unwrap().chunks.len();
        let mut tasks = Vec::new();
        for index in 0..chunk_count {
            let engine = self.clone_engine();
            let url = url.clone();
            let file = file.clone();
            let state = state.clone();
            let progress = progress.clone();
            tasks.push(tokio::spawn(async move {
                download_chunk(engine, url, file, state, progress, index).await
            }));
        }

        let mut result: Result<()> = Ok(());
        for task in tasks {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if result.is_ok() {
                        result = Err(error);
                    }
                }
                Err(join_error) => {
                    if result.is_ok() {
                        result = Err(anyhow!("download task panicked: {join_error}"));
                    }
                }
            }
        }

        saver.abort();
        let _ = saver.await;
        let snapshot = state.lock().unwrap().clone();
        let _ = snapshot.save(dest);

        result?;

        PartState::remove(dest);

        if self.verify
            && let Some(sha) = &expected_sha
        {
            progress.set_status(FileStatus::Verifying);
            if !verify::verify_sha256(dest, sha).await? {
                let _ = std::fs::remove_file(dest);
                PartState::remove(dest);
                bail!("sha256 mismatch for {}", remote.path);
            }
        }

        if size > 0 {
            progress.set_downloaded(size);
        }
        Ok(true)
    }

    fn clone_engine(&self) -> Arc<Engine> {
        // Engine fields are cheap to clone (Arcs); rebuilt as an Arc for chunk tasks.
        Arc::new(Engine {
            client: self.client.clone(),
            cancel: self.cancel.clone(),
            conn_sem: self.conn_sem.clone(),
            hub: self.hub.clone(),
            connections: self.connections,
            verify: self.verify,
            force: self.force,
        })
    }
}

async fn download_chunk(
    engine: Arc<Engine>,
    url: String,
    file: Arc<std::fs::File>,
    state: Arc<Mutex<PartState>>,
    progress: Arc<FileProgress>,
    index: usize,
) -> Result<()> {
    let mut attempt: u32 = 0;
    loop {
        if !engine.hub.wait_unpaused(&engine.cancel).await {
            bail!("cancelled");
        }
        let (start, len, done) = {
            let state = state.lock().unwrap();
            let chunk = &state.chunks[index];
            (offset_of(&state.chunks, index), chunk.len, chunk.done)
        };
        if len > 0 && done >= len {
            return Ok(());
        }
        let absolute_start = start + done;
        let range = if len > 0 {
            Some((absolute_start, start + len - 1))
        } else {
            None
        };

        match try_chunk(
            engine.clone(),
            &url,
            file.clone(),
            state.clone(),
            progress.clone(),
            index,
            absolute_start,
            range,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(error) => {
                if engine.cancel.is_cancelled() {
                    return Err(error);
                }
                attempt += 1;
                if attempt > MAX_RETRIES {
                    return Err(error).context("too many retries");
                }
                let backoff = Duration::from_millis(300 * 2u64.pow(attempt.min(5)));
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = engine.cancel.cancelled() => bail!("cancelled"),
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn try_chunk(
    engine: Arc<Engine>,
    url: &str,
    file: Arc<std::fs::File>,
    state: Arc<Mutex<PartState>>,
    progress: Arc<FileProgress>,
    index: usize,
    absolute_start: u64,
    range: Option<(u64, u64)>,
) -> Result<()> {
    let _permit = engine
        .conn_sem
        .clone()
        .acquire_owned()
        .await
        .context("connection semaphore closed")?;

    let response = engine.client.stream(url, range).await?;
    if range.is_some() && response.status() != StatusCode::PARTIAL_CONTENT {
        bail!("server ignored range request (HTTP {})", response.status());
    }

    let done0 = state.lock().unwrap().chunks[index].done;
    let mut stream = response.bytes_stream();
    let mut offset = absolute_start;

    loop {
        let next = tokio::select! {
            _ = engine.cancel.cancelled() => bail!("cancelled"),
            next = stream.next() => next,
        };
        let Some(item) = next else { break };
        if !engine.hub.wait_unpaused(&engine.cancel).await {
            bail!("cancelled");
        }
        let bytes: Bytes = item.context("stream error")?;
        let length = bytes.len() as u64;
        write_bytes(file.clone(), bytes, offset).await?;
        offset += length;

        let done = done0 + (offset - absolute_start);
        state.lock().unwrap().chunks[index].done = done;
        progress.add(length);

        let total = state.lock().unwrap().chunks[index].len;
        if total > 0 && done >= total {
            break;
        }
    }

    let (done, len) = {
        let state = state.lock().unwrap();
        (state.chunks[index].done, state.chunks[index].len)
    };
    if len > 0 && done < len {
        bail!("incomplete chunk {}/{}", done, len);
    }
    Ok(())
}

async fn write_bytes(file: Arc<std::fs::File>, data: Bytes, offset: u64) -> Result<()> {
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        #[cfg(unix)]
        use std::os::unix::fs::FileExt;
        #[cfg(windows)]
        use std::os::windows::fs::FileExt;

        let mut written = 0usize;
        while written < data.len() {
            #[cfg(unix)]
            let count = file.write_at(&data[written..], offset + written as u64)?;
            #[cfg(windows)]
            let count = file.seek_write(&data[written..], offset + written as u64)?;
            if count == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "short write",
                ));
            }
            written += count;
        }
        Ok(())
    })
    .await
    .context("write task panicked")??;
    Ok(())
}

fn offset_of(chunks: &[ChunkState], index: usize) -> u64 {
    chunks[..index].iter().map(|chunk| chunk.len).sum()
}

fn etag_compatible(previous: &Option<String>, current: &Option<String>) -> bool {
    match (previous, current) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

/// Split `size` into aligned chunks, one per connection where possible.
pub fn plan_chunks(size: u64, connections: usize) -> Vec<ChunkState> {
    if size == 0 || size <= MIN_CHUNK {
        return vec![ChunkState {
            index: 0,
            len: size,
            done: 0,
        }];
    }
    let max_chunks = size.div_ceil(MIN_CHUNK).max(1) as usize;
    let count = connections.clamp(1, 64).min(max_chunks);
    let base = size.div_ceil(count as u64);
    let chunk_size = base.div_ceil(MIN_CHUNK) * MIN_CHUNK;

    let mut chunks = Vec::new();
    let mut offset = 0u64;
    let mut index = 0u64;
    while offset < size {
        let len = chunk_size.min(size - offset);
        chunks.push(ChunkState {
            index,
            len,
            done: 0,
        });
        offset += len;
        index += 1;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_planning_covers_file() {
        let size = 100 * 1024 * 1024;
        let chunks = plan_chunks(size, 8);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.iter().map(|c| c.len).sum::<u64>(), size);
        let mut expected_start = 0;
        for (index, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.index, index as u64);
            assert_eq!(offset_of(&chunks, index), expected_start);
            expected_start += chunk.len;
        }
    }

    #[test]
    fn small_files_are_single_chunk() {
        assert_eq!(plan_chunks(1024, 8).len(), 1);
    }
}
