use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio_util::sync::CancellationToken;

use crate::api::{HfClient, RemoteFile, RepoInfo};
use crate::cli::{
    AnalyzeArgs, Cli, ConfigAction, ConfigArgs, DownloadArgs, InfoArgs, ListArgs, SearchArgs,
};
use crate::config::{self, Config};
use crate::download::{self, DownloadPlan, RunConfig};
use crate::progress::{self, FileProgress, ProgressHub};
use crate::store;
use crate::tui::{self, App, picker::PickItem};
use crate::util;

pub struct Settings {
    pub output_root: PathBuf,
    pub endpoint: String,
    pub token: Option<String>,
    pub proxy: Option<String>,
    pub connections: usize,
    pub max_active: usize,
    pub verify: bool,
    pub tui: bool,
    pub revision: String,
}

impl Settings {
    pub fn new(cli: &Cli, config: &Config) -> Self {
        Self {
            output_root: cli
                .output
                .clone()
                .map(|value| util::expand_tilde(&value))
                .unwrap_or_else(|| config.output_path()),
            endpoint: cli.endpoint.clone().unwrap_or_else(|| config.endpoint()),
            token: cli.token.clone().or_else(|| config.token.clone()),
            proxy: cli.proxy.clone().or_else(|| config.proxy.clone()),
            connections: cli.connections.unwrap_or(config.connections).clamp(1, 64),
            max_active: cli.max_active.unwrap_or(config.max_active).clamp(1, 64),
            verify: cli.verify || config.verify,
            tui: (config.tui || cli.tui) && !cli.no_tui,
            revision: cli.revision.clone().unwrap_or_else(|| "main".to_string()),
        }
    }

    pub fn client(&self) -> Result<Arc<HfClient>> {
        Ok(Arc::new(HfClient::new(
            &self.endpoint,
            self.token.clone(),
            self.proxy.clone(),
        )?))
    }
}

pub async fn download(settings: &Settings, args: &DownloadArgs) -> Result<()> {
    let (repo, quants) = crate::api::filter::parse_source(&args.source);
    let repo_type = if args.dataset { "datasets" } else { "models" };
    validate_repo(&repo)?;

    let client = settings.client()?;
    let info = client.tree(repo_type, &repo, &settings.revision).await?;

    let filter = crate::api::Filter {
        include: args.filter.clone(),
        exclude: args.exclude.clone(),
        quants,
    };
    let files: Vec<RemoteFile> = info
        .files
        .iter()
        .filter(|file| filter.matches(&file.path))
        .cloned()
        .collect();

    if files.is_empty() {
        bail!("no files in {repo} matched the given filters");
    }

    if args.dry_run {
        print_plan(&info, &files);
        return Ok(());
    }

    let command = std::env::args().collect::<Vec<_>>().join(" ");
    execute(settings, client, &info, files, command, args.json).await
}

pub async fn analyze(settings: &Settings, args: &AnalyzeArgs) -> Result<()> {
    let (repo, _) = crate::api::filter::parse_source(&args.source);
    let repo_type = if args.dataset { "datasets" } else { "models" };
    validate_repo(&repo)?;

    let client = settings.client()?;
    let info = client.tree(repo_type, &repo, &settings.revision).await?;
    let gguf: Vec<RemoteFile> = info.files.iter().filter(|f| f.is_gguf()).cloned().collect();

    if args.json {
        let payload = serde_json::json!({
            "repo": info.id,
            "commit": info.commit,
            "files": info.files.iter().map(|file| serde_json::json!({
                "path": file.path,
                "size": file.size,
                "quant": crate::api::gguf_quant(&file.path),
                "sha256": file.sha256,
            })).collect::<Vec<_>>(),
            "total_size": info.total_size(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if gguf.is_empty() {
        print_repo_summary(&info);
        println!("\nNo GGUF files found — listing all files:");
        print_plan(&info, &info.files);
        return Ok(());
    }

    let interactive = (args.interactive || args.download) && tui::use_tui(settings.tui);
    if !interactive {
        print_gguf_table(&info.id, &gguf);
        if args.download {
            let command = std::env::args().collect::<Vec<_>>().join(" ");
            execute(settings, client, &info, gguf, command, false).await?;
        }
        return Ok(());
    }

    let mut items: Vec<PickItem> = gguf
        .iter()
        .map(|file| {
            PickItem::new(
                file.path.clone(),
                crate::api::gguf_quant(&file.path),
                file.size,
            )
        })
        .collect();

    let selection = tui::picker::pick(&mut items, &info.id)?;
    let Some(selection) = selection else {
        println!("Cancelled.");
        return Ok(());
    };
    if selection.is_empty() {
        println!("Nothing selected.");
        return Ok(());
    }
    let files: Vec<RemoteFile> = selection.iter().map(|index| gguf[*index].clone()).collect();
    let command = std::env::args().collect::<Vec<_>>().join(" ");
    execute(settings, client, &info, files, command, false).await
}

pub async fn search(settings: &Settings, args: &SearchArgs) -> Result<()> {
    let repo_type = if args.dataset { "datasets" } else { "models" };
    let client = settings.client()?;
    let hits = client.search(repo_type, &args.query, args.limit).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
        return Ok(());
    }
    if hits.is_empty() {
        println!("No results for “{}”.", args.query);
        return Ok(());
    }
    println!(
        "{:<52} {:>10} {:>7}  pipeline",
        "repository", "downloads", "likes"
    );
    for hit in &hits {
        println!(
            "{:<52} {:>10} {:>7}  {}",
            util::truncate_end(&hit.id, 52),
            hit.downloads,
            hit.likes,
            hit.pipeline_tag.clone().unwrap_or_default()
        );
    }
    Ok(())
}

pub async fn list(settings: &Settings, args: &ListArgs) -> Result<()> {
    let manifests = store::find_manifests(&settings.output_root);
    if args.json {
        let payload: Vec<_> = manifests
            .iter()
            .map(|(path, manifest)| {
                serde_json::json!({
                    "path": path,
                    "repo": manifest.repo,
                    "revision": manifest.revision,
                    "commit": manifest.commit,
                    "downloaded_at": manifest.downloaded_at,
                    "files": manifest.files.len(),
                    "total_size": manifest.total_size,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if manifests.is_empty() {
        println!(
            "Nothing downloaded yet into {}.",
            util::display_path(&settings.output_root)
        );
        return Ok(());
    }
    println!(
        "{:<48} {:>6} {:>12}  downloaded",
        "repository", "files", "size"
    );
    let mut total: u64 = 0;
    for (_, manifest) in &manifests {
        total += manifest.total_size;
        println!(
            "{:<48} {:>6} {:>12}  {}",
            util::truncate_end(&manifest.repo, 48),
            manifest.files.len(),
            util::format_bytes(manifest.total_size),
            manifest.downloaded_at
        );
    }
    println!(
        "\n{} repositories · {}",
        manifests.len(),
        util::format_bytes(total)
    );
    Ok(())
}

pub async fn info(settings: &Settings, args: &InfoArgs) -> Result<()> {
    let manifests = store::find_manifests(&settings.output_root);
    let needle = args.query.to_ascii_lowercase();
    let matches: Vec<_> = manifests
        .iter()
        .filter(|(_, manifest)| manifest.repo.to_ascii_lowercase().contains(&needle))
        .collect();

    if matches.is_empty() {
        bail!("no downloaded repository matches “{}”", args.query);
    }
    if args.json {
        let payload: Vec<_> = matches
            .iter()
            .map(|(path, manifest)| {
                serde_json::json!({
                    "path": path,
                    "manifest": manifest,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    for (path, manifest) in matches {
        println!("{}", manifest.repo);
        println!(
            "  path        {}",
            util::display_path(path.parent().unwrap_or(path))
        );
        println!("  revision    {} @ {}", manifest.revision, manifest.commit);
        println!("  endpoint    {}", manifest.endpoint);
        println!("  downloaded  {}", manifest.downloaded_at);
        println!("  size        {}", util::format_bytes(manifest.total_size));
        println!("  files       {}", manifest.files.len());
        for file in &manifest.files {
            println!("    - {} ({})", file.path, util::format_bytes(file.size));
        }
        println!();
    }
    Ok(())
}

pub fn config_cmd(args: &ConfigArgs) -> Result<()> {
    match &args.action {
        None | Some(ConfigAction::Show) => {
            let config = config::load();
            let path = config::config_path();
            println!("# {}", path.display());
            let mut text = config;
            if text.token.is_some() {
                text.token = Some("***".to_string());
            }
            println!("{}", toml::to_string_pretty(&text)?);
        }
        Some(ConfigAction::Path) => {
            println!("{}", config::config_path().display());
        }
        Some(ConfigAction::Set { key, value }) => {
            let mut config = config::load();
            match key.as_str() {
                "output_dir" => config.output_dir = value.clone(),
                "endpoint" => config.endpoint = value.clone(),
                "token" => config.token = Some(value.clone()),
                "proxy" => config.proxy = Some(value.clone()),
                "connections" => {
                    config.connections = value.parse().context("connections must be a number")?
                }
                "max_active" => {
                    config.max_active = value.parse().context("max_active must be a number")?
                }
                "verify" => config.verify = parse_bool(value)?,
                "tui" => config.tui = parse_bool(value)?,
                other => bail!("unknown configuration key: {other}"),
            }
            let path = config::save(&config)?;
            println!("Updated {}", path.display());
        }
    }
    Ok(())
}

async fn execute(
    settings: &Settings,
    client: Arc<HfClient>,
    info: &RepoInfo,
    files: Vec<RemoteFile>,
    command: String,
    json: bool,
) -> Result<()> {
    let repo_path = store::repo_dir(&settings.output_root, &info.id);
    let hub_files: Vec<Arc<FileProgress>> = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            Arc::new(FileProgress::new(
                index,
                info.id.clone(),
                file.path.clone(),
                repo_path.join(&file.path),
                file.size,
            ))
        })
        .collect();
    let total: u64 = files.iter().map(|file| file.size).sum();
    let hub = Arc::new(ProgressHub::new(hub_files));
    let cancel = CancellationToken::new();
    let plan = DownloadPlan {
        repo: info.clone(),
        files: files.clone(),
        output_root: settings.output_root.clone(),
        command,
    };
    let run_config = RunConfig {
        connections: settings.connections,
        max_active: settings.max_active,
        verify: settings.verify,
        force: false,
    };

    if json {
        let download = tokio::spawn(download::run(
            client,
            plan,
            hub.clone(),
            cancel.clone(),
            run_config,
        ));
        let _ = download.await;
        print_json_summary(&hub);
        return Ok(());
    }

    if tui::use_tui(settings.tui) {
        hub.set_message(format!(
            "{} files · {} · {} connections/file",
            files.len(),
            util::format_bytes(total),
            settings.connections
        ));
        let sampler = progress::spawn_sampler(hub.clone(), cancel.clone());
        let download = tokio::spawn(download::run(
            client,
            plan,
            hub.clone(),
            cancel.clone(),
            run_config,
        ));
        let app = App::new(
            hub.clone(),
            settings.output_root.clone(),
            std::env::args().collect::<Vec<_>>().join(" "),
        );
        tui::run(app, cancel.clone(), download).await?;
        cancel.cancel();
        let _ = sampler.await;
    } else {
        run_plain(client, plan, hub.clone(), cancel.clone(), run_config).await;
    }

    if hub.failed_count() > 0 {
        bail!("{} file(s) failed", hub.failed_count());
    }
    Ok(())
}

async fn run_plain(
    client: Arc<HfClient>,
    plan: DownloadPlan,
    hub: Arc<ProgressHub>,
    cancel: CancellationToken,
    run_config: RunConfig,
) {
    let multi = MultiProgress::new();
    let overall_style = ProgressStyle::with_template(
        " {msg}\n {wide_bar} {pos:>3}% {bytes}/{total_bytes} {bytes_per_sec}",
    )
    .unwrap_or_else(|_| ProgressStyle::default_bar());
    let overall = multi.add(
        ProgressBar::new(100)
            .with_style(overall_style)
            .with_message("starting…"),
    );

    let bar_style =
        ProgressStyle::with_template(" {msg:<40} {wide_bar} {pos:>3}% {bytes}/{total_bytes}")
            .unwrap_or_else(|_| ProgressStyle::default_bar());

    let bars: Vec<ProgressBar> = plan
        .files
        .iter()
        .map(|file| {
            multi.add(
                ProgressBar::new(100)
                    .with_style(bar_style.clone())
                    .with_message(util::truncate_middle(file.file_name(), 40)),
            )
        })
        .collect();

    let renderer_token = cancel.child_token();
    let renderer_hub = hub.clone();
    let renderer_overall = overall.clone();
    let renderer_bars = bars.clone();
    let renderer = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = renderer_token.cancelled() => break,
                _ = ticker.tick() => {
                    renderer_overall.set_position(renderer_hub.percent() as u64);
                    renderer_overall.set_message(format!(
                        "{} / {} · {} · ETA {}",
                        util::format_bytes(renderer_hub.downloaded()),
                        util::format_bytes(renderer_hub.total_bytes),
                        util::format_speed(renderer_hub.speed() as f64),
                        renderer_hub.eta(),
                    ));
                    for (bar, file) in renderer_bars.iter().zip(renderer_hub.files.iter()) {
                        bar.set_position(file.percent() as u64);
                    }
                }
            }
        }
    });

    download::run(client, plan, hub.clone(), cancel.clone(), run_config).await;
    cancel.cancel();
    let _ = renderer.await;

    overall.set_position(100);
    overall.finish_with_message(format!(
        "done · {} in {}",
        util::format_bytes(hub.downloaded()),
        util::format_elapsed(hub.started.elapsed().as_secs_f64())
    ));
    for (bar, file) in bars.iter().zip(hub.files.iter()) {
        let status = file.status();
        if matches!(status, crate::progress::FileStatus::Failed(_)) {
            bar.abandon_with_message(format!("{} — {}", file.path, status.label()));
        } else {
            bar.finish();
        }
    }
}

fn print_json_summary(hub: &ProgressHub) {
    let files: Vec<serde_json::Value> = hub
        .files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.path,
                "size": file.total,
                "downloaded": file.progress(),
                "status": file.status().label(),
            })
        })
        .collect();
    let payload = serde_json::json!({
        "repo": hub.files.first().map(|file| file.repo.clone()).unwrap_or_default(),
        "total_bytes": hub.total_bytes,
        "downloaded_bytes": hub.downloaded(),
        "failed": hub.failed_count(),
        "files": files,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
}

fn print_plan(info: &RepoInfo, files: &[RemoteFile]) {
    println!("{} ({} @ {})", info.id, info.repo_type, info.commit);
    println!(
        "{} files · {}",
        files.len(),
        util::format_bytes(files.iter().map(|file| file.size).sum::<u64>())
    );
    for file in files {
        println!(
            "  {:<60} {:>12}",
            util::truncate_end(&file.path, 60),
            util::format_bytes(file.size)
        );
    }
}

fn print_repo_summary(info: &RepoInfo) {
    println!("{} ({})", info.id, info.repo_type);
    println!("commit   {}", info.commit);
    println!("files    {}", info.files.len());
    println!("size     {}", util::format_bytes(info.total_size()));
}

fn print_gguf_table(repo: &str, files: &[RemoteFile]) {
    println!("{repo} — {} GGUF files\n", files.len());
    println!("  {:<12} {:<6} {:>12}  file", "quant", "quality", "size");
    for file in files {
        let quant = crate::api::gguf_quant(&file.path).unwrap_or_else(|| "?".to_string());
        println!(
            "  {:<12} {:<6} {:>12}  {}",
            quant,
            crate::api::stars(&quant),
            util::format_bytes(file.size),
            file.path
        );
    }
}

fn validate_repo(repo: &str) -> Result<()> {
    if repo.is_empty()
        || repo.starts_with('/')
        || repo.ends_with('/')
        || repo.contains("//")
        || repo.chars().any(char::is_whitespace)
    {
        bail!("expected a repository name like org/name, got “{repo}”");
    }
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => bail!("expected true/false, got “{value}”"),
    }
}
