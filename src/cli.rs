use clap::{Args, Parser, Subcommand};

pub const KNOWN_COMMANDS: [&str; 7] = [
    "download", "analyze", "search", "list", "info", "config", "version",
];

#[derive(Parser, Debug)]
#[command(
    name = "hfd",
    version,
    author,
    about = "hfd — the best HuggingFace model downloader",
    long_about = "Fast, resumable, multi-connection downloads for HuggingFace models and datasets, \
                  with a live terminal dashboard and an interactive GGUF quantization picker.",
    propagate_version = true
)]
pub struct Cli {
    /// Output directory for flat model folders [default: ~/models]
    #[arg(
        short = 'o',
        long,
        global = true,
        value_name = "DIR",
        visible_alias = "local-dir"
    )]
    pub output: Option<String>,

    /// Parallel connections per file
    #[arg(short = 'c', long, global = true, value_name = "N")]
    pub connections: Option<usize>,

    /// Files downloaded at the same time
    #[arg(long = "max-active", global = true, value_name = "N")]
    pub max_active: Option<usize>,

    /// Branch, tag or commit
    #[arg(short = 'b', long, global = true, value_name = "REV")]
    pub revision: Option<String>,

    /// HuggingFace access token
    #[arg(short = 't', long, global = true, value_name = "TOKEN")]
    pub token: Option<String>,

    /// Proxy URL (http://, https://, socks5://, socks5h://)
    #[arg(long, global = true, value_name = "URL")]
    pub proxy: Option<String>,

    /// API endpoint, e.g. https://hf-mirror.com
    #[arg(long, global = true, value_name = "URL")]
    pub endpoint: Option<String>,

    /// Verify SHA-256 of downloaded files
    #[arg(long, global = true)]
    pub verify: bool,

    /// Disable the interactive TUI
    #[arg(long, global = true)]
    pub no_tui: bool,

    /// Force the interactive TUI (requires a terminal)
    #[arg(long, global = true)]
    pub tui: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Download a model or dataset (default command)
    Download(DownloadArgs),
    /// Inspect a repository before downloading
    Analyze(AnalyzeArgs),
    /// Search the HuggingFace Hub
    Search(SearchArgs),
    /// List everything downloaded into the output directory
    List(ListArgs),
    /// Show details of a downloaded repository
    Info(InfoArgs),
    /// Show or edit configuration
    Config(ConfigArgs),
    /// Show version information
    Version,
}

#[derive(Args, Debug)]
pub struct DownloadArgs {
    /// Repository as org/name, optionally with :quant (org/name:q4_k_m,q5_k_m)
    #[arg(value_name = "REPO[:QUANT]")]
    pub source: String,

    /// Only download files whose path contains one of these patterns
    #[arg(
        short = 'F',
        long = "filter",
        value_delimiter = ',',
        value_name = "PATTERN"
    )]
    pub filter: Vec<String>,

    /// Skip files whose path contains one of these patterns
    #[arg(
        short = 'E',
        long = "exclude",
        value_delimiter = ',',
        value_name = "PATTERN"
    )]
    pub exclude: Vec<String>,

    /// Treat the repository as a dataset
    #[arg(long)]
    pub dataset: bool,

    /// Only show what would be downloaded
    #[arg(long)]
    pub dry_run: bool,

    /// Re-download files even when they already look complete
    #[arg(long)]
    pub force: bool,

    /// Print machine-readable JSON instead of a live view
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct AnalyzeArgs {
    /// Repository as org/name, optionally with :quant
    #[arg(value_name = "REPO")]
    pub source: String,

    /// Open the interactive picker
    #[arg(short = 'i', long)]
    pub interactive: bool,

    /// Treat the repository as a dataset
    #[arg(long)]
    pub dataset: bool,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,

    /// Download the selection immediately (non-interactive picker)
    #[arg(long)]
    pub download: bool,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Query string
    #[arg(value_name = "QUERY")]
    pub query: String,

    /// Search datasets instead of models
    #[arg(long)]
    pub dataset: bool,

    /// Maximum number of results
    #[arg(short = 'l', long, default_value_t = 25)]
    pub limit: usize,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// Substring matched against downloaded repositories
    #[arg(value_name = "QUERY")]
    pub query: String,

    /// Print machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the configuration file path
    Path,
    /// Print the effective configuration
    Show,
    /// Set a configuration value
    Set {
        /// Key: output_dir, endpoint, token, connections, max_active, verify, proxy, tui
        key: String,
        value: String,
    },
}
