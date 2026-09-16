use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkState {
    pub index: u64,
    pub len: u64,
    pub done: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartState {
    pub version: u32,
    pub path: String,
    pub total: u64,
    pub etag: Option<String>,
    pub sha256: Option<String>,
    pub chunk_size: u64,
    pub chunks: Vec<ChunkState>,
    pub updated: String,
}

impl PartState {
    pub const VERSION: u32 = 1;

    pub fn new(
        path: &str,
        total: u64,
        etag: Option<String>,
        sha256: Option<String>,
        chunk_size: u64,
        chunks: Vec<ChunkState>,
    ) -> Self {
        Self {
            version: Self::VERSION,
            path: path.to_string(),
            total,
            etag,
            sha256,
            chunk_size,
            chunks,
            updated: crate::util::now_iso8601(),
        }
    }

    pub fn downloaded(&self) -> u64 {
        self.chunks.iter().map(|c| c.done).sum()
    }

    pub fn save(&self, dest: &Path) -> Result<()> {
        let sidecar = sidecar_path(dest);
        let text = serde_json::to_string(self)?;
        let tmp = sidecar.with_extension("hfd-part.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &sidecar)?;
        Ok(())
    }

    pub fn load(dest: &Path) -> Option<PartState> {
        let sidecar = sidecar_path(dest);
        let text = std::fs::read_to_string(sidecar).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn remove(dest: &Path) {
        let _ = std::fs::remove_file(sidecar_path(dest));
    }
}

pub fn sidecar_path(dest: &Path) -> PathBuf {
    let mut value = dest.as_os_str().to_owned();
    value.push(".hfd-part");
    PathBuf::from(value)
}
