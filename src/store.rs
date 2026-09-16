use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const MANIFEST_NAME: &str = "hfd.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub repo: String,
    pub repo_type: String,
    pub revision: String,
    pub commit: String,
    pub endpoint: String,
    pub downloaded_at: String,
    pub command: String,
    pub total_size: u64,
    pub files: Vec<ManifestFile>,
}

pub fn repo_dir(root: &Path, repo: &str) -> PathBuf {
    root.join(repo)
}

pub fn manifest_path(repo_path: &Path) -> PathBuf {
    repo_path.join(MANIFEST_NAME)
}

pub fn write_manifest(repo_path: &Path, manifest: &Manifest) -> Result<()> {
    std::fs::create_dir_all(repo_path)
        .with_context(|| format!("creating {}", repo_path.display()))?;
    let path = manifest_path(repo_path);
    let text = serde_yaml_like(manifest);
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Serialize the manifest as TOML-flavored text that is still valid YAML-ish,
/// but simple and stable. We keep `.yaml` naming for familiarity.
fn serde_yaml_like(manifest: &Manifest) -> String {
    let mut out = String::new();
    out.push_str(&format!("repo: {}\n", manifest.repo));
    out.push_str(&format!("repo_type: {}\n", manifest.repo_type));
    out.push_str(&format!("revision: {}\n", manifest.revision));
    out.push_str(&format!("commit: {}\n", manifest.commit));
    out.push_str(&format!("endpoint: {}\n", manifest.endpoint));
    out.push_str(&format!("downloaded_at: {}\n", manifest.downloaded_at));
    out.push_str(&format!("total_size: {}\n", manifest.total_size));
    out.push_str(&format!("command: {}\n", manifest.command));
    out.push_str("files:\n");
    for file in &manifest.files {
        out.push_str(&format!("  - path: {}\n", file.path));
        out.push_str(&format!("    size: {}\n", file.size));
        if let Some(sha) = &file.sha256 {
            out.push_str(&format!("    sha256: {sha}\n"));
        }
    }
    out
}

/// Parse a manifest file written by [`write_manifest`].
pub fn read_manifest(path: &Path) -> Result<Manifest> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_manifest(&text)
}

pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let mut manifest = Manifest {
        repo: String::new(),
        repo_type: "models".to_string(),
        revision: String::new(),
        commit: String::new(),
        endpoint: String::new(),
        downloaded_at: String::new(),
        command: String::new(),
        total_size: 0,
        files: Vec::new(),
    };
    let mut current: Option<ManifestFile> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("  - path: ") {
            if let Some(file) = current.take() {
                manifest.files.push(file);
            }
            current = Some(ManifestFile {
                path: rest.trim().to_string(),
                size: 0,
                sha256: None,
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("    size: ") {
            if let Some(file) = current.as_mut() {
                file.size = rest.trim().parse().unwrap_or(0);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("    sha256: ") {
            if let Some(file) = current.as_mut() {
                file.sha256 = Some(rest.trim().to_string());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(": ") {
            let value = value.trim().to_string();
            match key {
                "repo" => manifest.repo = value,
                "repo_type" => manifest.repo_type = value,
                "revision" => manifest.revision = value,
                "commit" => manifest.commit = value,
                "endpoint" => manifest.endpoint = value,
                "downloaded_at" => manifest.downloaded_at = value,
                "total_size" => manifest.total_size = value.parse().unwrap_or(0),
                "command" => manifest.command = value,
                _ => {}
            }
        }
    }
    if let Some(file) = current.take() {
        manifest.files.push(file);
    }
    Ok(manifest)
}

/// Find every manifest under `root`.
pub fn find_manifests(root: &Path) -> Vec<(PathBuf, Manifest)> {
    let mut found = Vec::new();
    walk(root, &mut found, 0);
    found.sort_by(|a, b| a.1.repo.cmp(&b.1.repo));
    found
}

fn walk(dir: &Path, found: &mut Vec<(PathBuf, Manifest)>, depth: usize) {
    if depth > 6 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found, depth + 1);
        } else if path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_NAME)
            && let Ok(manifest) = read_manifest(&path)
        {
            found.push((path, manifest));
        }
    }
}
