use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::header::{ACCEPT, AUTHORIZATION, RANGE};
use reqwest::{Client, Response, StatusCode};

use super::repo::{LfsInfo, RemoteFile, RepoInfo, SearchHit, TreeEntry};

pub const USER_AGENT_VALUE: &str = concat!("hfd/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct HfClient {
    client: Client,
    endpoint: String,
    token: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub supports_ranges: bool,
    pub size: Option<u64>,
    pub etag: Option<String>,
}

impl HfClient {
    pub fn new(endpoint: &str, token: Option<String>, proxy: Option<String>) -> Result<Self> {
        let mut builder = Client::builder()
            .user_agent(USER_AGENT_VALUE)
            .connect_timeout(Duration::from_secs(20))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_nodelay(true)
            .redirect(reqwest::redirect::Policy::limited(10));

        if let Some(proxy) = proxy.filter(|p| !p.trim().is_empty()) {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy.trim())
                    .with_context(|| format!("invalid proxy URL: {proxy}"))?,
            );
        }

        let client = builder.build().context("building HTTP client")?;
        Ok(Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            token,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn auth(&self, mut request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.token {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        request
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api/{}", self.endpoint, encode_segments(path))
    }

    /// URL for downloading a file from a repo (resolve endpoint, follows redirects).
    pub fn resolve_url(&self, repo_type: &str, repo: &str, revision: &str, path: &str) -> String {
        let prefix = if repo_type == "datasets" {
            "/datasets"
        } else {
            ""
        };
        let mut url = format!("{}{}/{}", self.endpoint, prefix, encode_segments(repo));
        url.push_str("/resolve/");
        url.push_str(&encode_segment(revision));
        url.push('/');
        url.push_str(&encode_segments(path));
        url
    }

    /// List every file in a repository, following pagination.
    pub async fn tree(&self, repo_type: &str, repo: &str, revision: &str) -> Result<RepoInfo> {
        let mut next: Option<String> = Some(self.tree_url(repo_type, repo, revision, None));
        let mut files = Vec::new();
        let mut commit = String::new();

        while let Some(url) = next {
            let response = self
                .auth(self.client.get(&url).header(ACCEPT, "application/json"))
                .send()
                .await
                .with_context(|| format!("listing {repo}"))?;

            if response.status() == StatusCode::UNAUTHORIZED
                || response.status() == StatusCode::FORBIDDEN
            {
                bail!(
                    "access denied for {repo} (status {}). Set a token with --token or HF_TOKEN \
                     and make sure the license is accepted on huggingface.co",
                    response.status()
                );
            }
            if response.status() == StatusCode::NOT_FOUND {
                bail!("repository not found: {repo} (revision: {revision})");
            }
            if !response.status().is_success() {
                bail!("failed to list {repo}: HTTP {}", response.status());
            }

            if commit.is_empty()
                && let Some(value) = response.headers().get("x-repo-commit")
            {
                commit = value.to_str().unwrap_or_default().to_string();
            }

            next = link_next(response.headers());

            let entries: Vec<TreeEntry> = response
                .json()
                .await
                .with_context(|| format!("parsing file list for {repo}"))?;

            for entry in entries {
                if entry.kind != "file" {
                    continue;
                }
                let (size, sha256) = match &entry.lfs {
                    Some(LfsInfo {
                        oid: Some(oid),
                        size,
                    }) => (size.unwrap_or(entry.size), Some(oid.clone())),
                    _ => (entry.size, None),
                };
                files.push(RemoteFile {
                    path: entry.path,
                    size,
                    sha256,
                    oid: entry.oid,
                });
            }
        }

        if files.is_empty() {
            bail!("repository {repo} contains no files at revision {revision}");
        }

        if commit.is_empty() {
            commit = self
                .revision_sha(repo_type, repo, revision)
                .await
                .unwrap_or_default();
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(RepoInfo {
            id: repo.to_string(),
            repo_type: repo_type.to_string(),
            revision: revision.to_string(),
            commit,
            files,
        })
    }

    fn tree_url(
        &self,
        repo_type: &str,
        repo: &str,
        revision: &str,
        cursor: Option<&str>,
    ) -> String {
        let mut url = self.api_url(&format!("{repo_type}/{repo}/tree/{revision}"));
        url.push_str("?recursive=true&expand=false");
        if let Some(cursor) = cursor {
            url.push_str("&cursor=");
            url.push_str(&encode_segment(cursor));
        }
        url
    }

    /// Commit sha for a repository revision.
    pub async fn revision_sha(
        &self,
        repo_type: &str,
        repo: &str,
        revision: &str,
    ) -> Result<String> {
        let url = self.api_url(&format!("{repo_type}/{repo}/revision/{revision}"));
        let response = self
            .auth(self.client.get(&url).header(ACCEPT, "application/json"))
            .send()
            .await?;
        if !response.status().is_success() {
            return Ok(String::new());
        }
        let value: serde_json::Value = response.json().await.unwrap_or_default();
        Ok(value
            .get("sha")
            .and_then(|sha| sha.as_str())
            .unwrap_or_default()
            .to_string())
    }

    /// Search models or datasets.
    pub async fn search(
        &self,
        repo_type: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let mut url = format!(
            "{}?search={}",
            self.api_url(repo_type),
            encode_segment(query)
        );
        url.push_str(&format!(
            "&limit={limit}&full=false&sort=downloads&direction=-1"
        ));
        let response = self
            .auth(self.client.get(&url).header(ACCEPT, "application/json"))
            .send()
            .await
            .context("searching the hub")?;
        if !response.status().is_success() {
            bail!("search failed: HTTP {}", response.status());
        }
        let hits: Vec<SearchHit> = response.json().await.context("parsing search results")?;
        Ok(hits)
    }

    /// Probe a file for range support, size and etag.
    pub async fn probe(&self, url: &str) -> Result<Probe> {
        let response = self
            .auth(self.client.get(url).header(RANGE, "bytes=0-0"))
            .send()
            .await
            .with_context(|| format!("probing {url}"))?;

        if !response.status().is_success() {
            bail!("failed to download (HTTP {})", response.status());
        }

        let supports_ranges = response.status() == StatusCode::PARTIAL_CONTENT;
        let mut probe = Probe {
            supports_ranges,
            ..Default::default()
        };

        if supports_ranges {
            if let Some(total) = response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|v| v.trim().parse::<u64>().ok())
            {
                probe.size = Some(total);
            }
        } else {
            probe.size = response.content_length();
        }

        probe.etag = etag_of(response.headers());
        Ok(probe)
    }

    /// Start a streaming GET request with an optional inclusive byte range.
    pub async fn stream(&self, url: &str, range: Option<(u64, u64)>) -> Result<Response> {
        let mut request = self.client.get(url);
        if let Some((start, end)) = range {
            request = request.header(RANGE, format!("bytes={start}-{end}"));
        }
        let response = self.auth(request).send().await.context("request failed")?;

        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            bail!(
                "access denied (HTTP {}) — set a token with --token or HF_TOKEN",
                response.status()
            );
        }
        if !response.status().is_success() {
            bail!("download failed with HTTP {}", response.status());
        }
        Ok(response)
    }
}

pub fn etag_of(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get("x-linked-etag")
        .or_else(|| headers.get(reqwest::header::ETAG))
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim_matches('"').to_string())
}

fn link_next(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let value = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    for part in value.split(',') {
        if !part.contains("rel=\"next\"") {
            continue;
        }
        let start = part.find('<')? + 1;
        let end = part.find('>')?;
        if end > start {
            return Some(part[start..end].to_string());
        }
    }
    None
}

fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn encode_segments(path: &str) -> String {
    path.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}
