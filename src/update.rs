use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};

use crate::config;

/// GitHub repository that publishes hfd releases.
const DEFAULT_REPO: &str = "iamhsouna/hfd";
const TARGET: &str = env!("HFD_TARGET");
const VERSION: &str = env!("HFD_VERSION");

#[derive(Debug, Clone, Default)]
pub struct UpdateOptions {
    pub check_only: bool,
    pub force: bool,
    pub version: Option<String>,
}

pub async fn run(options: UpdateOptions) -> Result<()> {
    let config = config::load();
    let repo = std::env::var("HFD_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string());
    let client = build_client(config.proxy.as_deref())?;

    println!("hfd {VERSION} ({TARGET})");
    println!("checking {repo} for updates…");

    let release = fetch_release(&client, &repo, options.version.as_deref()).await?;
    let tag = release
        .get("tag_name")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let latest = tag.trim_start_matches('v').to_string();

    if latest.is_empty() {
        bail!("release has no tag name");
    }

    let current = parse_version(VERSION);
    let available = parse_version(&latest);
    let newer = available > current;

    if !newer && !options.force {
        println!("hfd is already up to date (latest: {latest}).");
        return Ok(());
    }

    println!(
        "{} available: {latest} (current: {VERSION})",
        if options.force { "release" } else { "update" }
    );

    let assets = release
        .get("assets")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let asset_name = format!("hfd-{TARGET}");
    let asset = find_asset(&assets, &asset_name).with_context(|| {
        format!(
            "no prebuilt binary for {TARGET} in release {tag}; \
             build from source with the installer (--from-source)"
        )
    })?;
    let download_url = asset
        .get("browser_download_url")
        .and_then(|value| value.as_str())
        .context("release asset is missing a download URL")?;

    if options.check_only {
        println!("update available: {download_url}");
        return Ok(());
    }

    let bytes = download(&client, download_url).await?;

    if let Some(checksum) = find_asset(&assets, &format!("{asset_name}.sha256"))
        && let Some(url) = checksum
            .get("browser_download_url")
            .and_then(|value| value.as_str())
        && let Ok(text) = client.get(url).send().await?.text().await
    {
        let expected = text.split_whitespace().next().unwrap_or_default();
        let actual = sha256_hex(&bytes);
        if !expected.is_empty() && !expected.eq_ignore_ascii_case(&actual) {
            bail!("checksum mismatch: expected {expected}, got {actual}");
        }
        println!("checksum verified");
    }

    install_binary(&bytes).context(
        "could not replace the running binary — re-run the installer, \
         or write to a directory you own with --bin-dir",
    )?;

    println!("updated hfd to {latest}");
    Ok(())
}

fn build_client(proxy: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("hfd/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(300));
    if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
        builder = builder.proxy(reqwest::Proxy::all(proxy.trim())?);
    }
    Ok(builder.build()?)
}

async fn fetch_release(
    client: &reqwest::Client,
    repo: &str,
    version: Option<&str>,
) -> Result<serde_json::Value> {
    let url = match version {
        Some(version) => {
            let tag = if version.starts_with('v') {
                version.to_string()
            } else {
                format!("v{version}")
            };
            format!("https://api.github.com/repos/{repo}/releases/tags/{tag}")
        }
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };

    let mut request = client
        .get(&url)
        .header("Accept", "application/vnd.github+json");
    if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN"))
        && !token.is_empty()
    {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    let response = request.send().await.context("contacting GitHub")?;
    match response.status().as_u16() {
        200 => {}
        404 => bail!(
            "no release {} found for {repo}",
            version.unwrap_or("(latest)")
        ),
        403 => bail!("GitHub API rate limit reached — set GITHUB_TOKEN and retry"),
        other => bail!("GitHub returned HTTP {other}"),
    }
    Ok(response.json().await?)
}

fn find_asset<'a>(assets: &'a [serde_json::Value], name: &str) -> Option<&'a serde_json::Value> {
    assets.iter().find(|asset| {
        asset
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value == name)
            .unwrap_or(false)
    })
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send().await.context("downloading update")?;
    if !response.status().is_success() {
        bail!("download failed with HTTP {}", response.status());
    }
    let total = response.content_length().unwrap_or(0);
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template(" {msg:<24} {wide_bar} {bytes}/{total_bytes}")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );
    bar.set_message("downloading update");

    let mut bytes = Vec::with_capacity(total as usize);
    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        bytes.extend_from_slice(&chunk);
        bar.inc(chunk.len() as u64);
    }
    bar.finish_and_clear();
    Ok(bytes)
}

fn install_binary(bytes: &[u8]) -> Result<()> {
    let exe = std::env::current_exe().context("locating the running executable")?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let dir = exe.parent().context("executable has no parent directory")?;
    let temp: PathBuf = dir.join(format!(".hfd-update-{}", std::process::id()));

    {
        let mut file =
            std::fs::File::create(&temp).with_context(|| format!("creating {}", temp.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))?;
    }

    std::fs::rename(&temp, &exe).with_context(|| {
        let _ = std::fs::remove_file(&temp);
        format!("replacing {}", exe.display())
    })?;
    Ok(())
}

fn parse_version(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .take_while(|part| part.chars().all(|c| c.is_ascii_digit()))
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(parse_version("0.2.0") > parse_version("0.1.9"));
        assert!(parse_version("v1.0.0") > parse_version("0.9.9"));
        assert_eq!(parse_version("0.1.0"), vec![0, 1, 0]);
    }

    #[test]
    fn checksum() {
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
