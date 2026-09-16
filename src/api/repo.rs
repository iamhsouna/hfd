use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFile {
    pub path: String,
    pub size: u64,
    /// SHA-256 from LFS metadata, when available. This is the content hash.
    pub sha256: Option<String>,
    /// Git object id (blob sha1) used to detect content changes.
    pub oid: Option<String>,
}

impl RemoteFile {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn is_gguf(&self) -> bool {
        self.path.to_ascii_lowercase().ends_with(".gguf")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub id: String,
    pub repo_type: String,
    pub revision: String,
    pub commit: String,
    pub files: Vec<RemoteFile>,
}

impl RepoInfo {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TreeEntry {
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub oid: Option<String>,
    #[serde(default)]
    pub lfs: Option<LfsInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LfsInfo {
    #[serde(default)]
    pub oid: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub id: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub pipeline_tag: Option<String>,
    #[serde(default, rename = "lastModified")]
    pub last_modified: Option<String>,
}
