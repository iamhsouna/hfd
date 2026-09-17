# hfd

**The fastest, smartest way to download models and datasets from the HuggingFace Hub.**

A single Rust binary with a real-time terminal dashboard, multi-connection
chunked downloads, crash-safe resume, SHA-256 verification, GGUF quantization
analysis, and an interactive quant picker.

```
hfd TheBloke/Mistral-7B-Instruct-v0.2-GGUF:q4_k_m
```

Files land in `~/models/<org>/<repo>/...` as plain, ready-to-use files.

---

## Highlights

- **Multi-connection downloads** — up to 64 parallel HTTP range requests per
  file, written with positional I/O (no locks, no gaps).
- **Concurrent files** — download several files at once with a global
  connection budget.
- **Resume anything** — a `<file>.hfd-part` sidecar records every chunk. Kill
  the process, lose the network, or reboot; the next run continues exactly
  where it stopped.
- **Integrity by default available** — stream SHA-256 verification against the
  LFS object hash with `--verify`.
- **Best-in-class TUI** — live overall gauge, per-file bars, speed, ETA, a
  navigable detail pane with a speed sparkline, pause/resume and cancel.
- **Interactive GGUF picker** — quality stars, RAM estimates, a “recommended”
  badge (Q4_K_M), multi-select, and one keystroke to download.
- **Datasets too** — full support for `datasets/` repositories.
- **Private & gated repos** — `--token` or `HF_TOKEN`.
- **Mirrors & proxies** — `--endpoint https://hf-mirror.com`, HTTP/SOCKS5
  proxies via `--proxy` or environment variables.

---

## Install

### One-liner (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/iamhsouna/hfd/master/install.sh | bash
```

The installer detects your OS and architecture, installs the required
dependencies, and puts `hfd` on your `PATH` (in `~/.local/bin` by default).
It works on macOS, Ubuntu/Debian, Arch Linux, Fedora, and openSUSE.

It prefers a prebuilt release binary and automatically falls back to building
from source if none is available for your platform.

```bash
# Build from source instead
curl -fsSL .../install.sh | bash -s -- --from-source

# Choose the install directory
curl -fsSL .../install.sh | bash -s -- --bin-dir /usr/local/bin

# Pin a release
curl -fsSL .../install.sh | bash -s -- --version v0.1.0
```

Installer flags: `--from-source`, `--version TAG`, `--bin-dir DIR`,
`--no-modify-path`, `--no-deps`, `--help`.

### From source

```bash
git clone https://github.com/iamhsouna/hfd && cd hfd
cargo build --release
install -m 755 target/release/hfd ~/.local/bin/hfd
```

Requirements: a recent stable Rust toolchain.

### Update

```bash
hfd update            # update to the latest release
hfd update --check    # only check, do not install
hfd update --tag v0.1.0
```

`hfd update` downloads the matching prebuilt binary from GitHub Releases,
verifies its checksum when available, and atomically replaces the running
executable.

---

## Quick start

```bash
# Download a whole model (default command)
hfd sshleifer/tiny-gpt2

# Download just one file (exact name, or any part of the path)
hfd download ISTA-DASLab/Qwen3.8-27B-GSQ-RCO-GGUF Qwen3.8-27B-GSQ-RCO-IQ3_S-mtp.gguf

# Or with an include pattern (-F matches any part of the path)
hfd download owner/repo -F q4_k_m
hfd download owner/repo -F IQ3_S-mtp.gguf

# Pick a quantization interactively
hfd analyze -i TheBloke/Mistral-7B-Instruct-v0.2-GGUF

# Download one quant directly
hfd TheBloke/Mistral-7B-Instruct-v0.2-GGUF:q4_k_m

# Several quants at once
hfd TheBloke/Mistral-7B-Instruct-v0.2-GGUF:q4_k_m,q5_k_m

# Preview without downloading
hfd download owner/repo --dry-run

# Verify hashes as you go
hfd download owner/repo --verify

# A dataset
hfd download squad --dataset

# Search the Hub
hfd search "code llama" -l 10
```

## Commands

| Command | Description |
| --- | --- |
| `download` | Download a model or dataset (the default command) |
| `analyze` | Inspect a repository; `-i` opens the GGUF picker |
| `search` | Search models or datasets |
| `list` | List everything downloaded under the output directory |
| `info` | Show details of a downloaded repository |
| `config` | Show, edit, or locate the configuration file |
| `update` | Update hfd to the latest release (`--check`, `--force`, `--tag`) |
| `version` | Show version information |

## Global options

| Flag | Default | Description |
| --- | --- | --- |
| `-o, --output <DIR>` | `~/models` | Output directory (`--local-dir` is an alias) |
| `-c, --connections <N>` | `8` | Parallel connections per file |
| `--max-active <N>` | `3` | Files downloaded concurrently |
| `-b, --revision <REV>` | `main` | Branch, tag, or commit |
| `-t, --token <TOKEN>` | — | HuggingFace access token |
| `--proxy <URL>` | — | `http://`, `https://`, `socks5://`, `socks5h://` |
| `--endpoint <URL>` | `https://huggingface.co` | API endpoint / mirror |
| `--verify` | off | Verify SHA-256 of downloaded files |
| `--no-tui` / `--tui` | auto | Disable / force the dashboard |

`download` also accepts `-F/--filter` (include patterns), `-E/--exclude`,
`--dataset`, `--dry-run`, `--force`, and `--json`.

`analyze` accepts `-i/--interactive`, `--dataset`, `--json`, and `--download`.

## Storage layout

```
~/models/
└── TheBloke/
    └── Mistral-7B-Instruct-v0.2-GGUF/
        ├── mistral-7b-instruct-v0.2.Q4_K_M.gguf
        └── hfd.yaml          # manifest: repo, commit, files, sizes, hashes
```

Every successful download writes an `hfd.yaml` manifest. `hfd list` and
`hfd info` read these to report what you have and where.

## Resume

Interrupted transfers leave a `<file>.hfd-part` sidecar next to the partial
file. It records the file size, etag, and the byte offset of every chunk.

```bash
hfd download owner/repo
# ... interrupt ...

hfd download owner/repo     # picks up exactly where it stopped
```

If the remote file changed (different size or etag), the partial state is
discarded and the download restarts cleanly.

## TUI keys

| Key | Action |
| --- | --- |
| `q` / `Esc` | Quit (saves resume state) |
| `p` / `Space` | Pause / resume all transfers |
| `↑` `↓` / `k` `j` | Select a file |
| `g` / `G` | Jump to first / last |
| `c` | Cancel all downloads |
| `o` | Open the output folder |
| `?` / `h` | Toggle help |

## Configuration

`~/.config/hfd/config.toml` (created on `hfd config set`):

```toml
output_dir = "~/models"
endpoint = "https://huggingface.co"
connections = 8
max_active = 3
verify = false
tui = true
# token = "hf_xxx"
# proxy = "http://proxy:8080"
```

```bash
hfd config show
hfd config set connections 16
hfd config path
```

Environment overrides: `HF_TOKEN`, `HF_ENDPOINT`, `HFD_OUTPUT_DIR`,
`HTTP_PROXY` / `HTTPS_PROXY`.

## How it works

1. The repository tree is fetched from `/api/{models|datasets}/{repo}/tree/{rev}`
   with pagination, yielding each file’s size and LFS SHA-256.
2. Each file is probed with a `Range: bytes=0-0` request to detect range
   support and the exact size/etag.
3. Files larger than 4 MiB are split into aligned chunks (one per connection)
   and fetched concurrently with `Range` requests; every write is a positional
   `pwrite`, so chunks never interfere.
4. Partial state is flushed to the `.hfd-part` sidecar once per second and on
   every exit path, making crashes cheap.
5. When every chunk is present the sidecar is removed; with `--verify` the
   file is streamed through SHA-256 and compared to the LFS hash.

## Development

```bash
cargo test      # unit tests
cargo clippy    # lints
cargo fmt       # formatting
```

## License

Apache-2.0.
