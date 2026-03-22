# supp

Structured code context for LLMs. Extracts files, diffs, symbols, and trees from your codebase and copies them to the clipboard — ready to paste into any chat.

[![CI](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml/badge.svg)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/AndrewPBerg/supp/branch/main/graph/badge.svg)](https://codecov.io/gh/AndrewPBerg/supp)

## Install

```sh
# From crates.io
cargo install supp

# Or via install script (Linux / macOS)
curl -fsSL https://raw.githubusercontent.com/AndrewPBerg/supp/main/install.sh | bash
```

**Windows (PowerShell):**

```powershell
Invoke-WebRequest -Uri https://raw.githubusercontent.com/AndrewPBerg/supp/main/install.ps1 -OutFile install.ps1
.\install.ps1
Remove-Item install.ps1
```

Binaries are available for Linux (x86_64, ARM64, musl), macOS (Intel, Apple Silicon), and Windows.
See [GitHub Releases](https://github.com/AndrewPBerg/supp/releases) for downloads.

## Quick start

```sh
# Get context from files — copies to clipboard automatically
supp src/main.rs

# Multiple files and directories
supp src/ Cargo.toml

# See what changed on your branch
supp diff

# Show the project tree with git status
supp tree

# Search for a symbol by name
supp sym UserService

# Deep-dive a symbol — definition, call sites, dependencies
supp why handle_request

# Pick files interactively with fzf
supp pick
```

Add `-n` to any command to print output without copying to clipboard.

## Commands

| Command | What it does |
|---------|-------------|
| `supp <paths>` | Extract file contents with token estimate |
| `supp diff` | Structured diff between branches |
| `supp tree` | Directory tree with git status markers |
| `supp sym <query>` | Search symbols with PageRank ranking |
| `supp why <symbol>` | Full context for a symbol |
| `supp pick` | Interactive file picker (requires fzf) |
| `supp clean-cache` | Delete the symbol cache for a project |

> **NOTE:** `supp pick` requires [fzf](https://github.com/junegunn/fzf) to be installed and available on your `PATH`. Install it via your package manager (e.g. `brew install fzf`, `winget install fzf`, `pacman -S fzf`, `xbps-install fzf`) before using this command.
| `supp completions <shell>` | Generate shell completions (bash, zsh, fish) |
| `supp mcp` | Start as an MCP server |

## Useful flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
| `--json` | | Output as JSON (machine-readable) |
| `--regex` | `-r` | Filter paths by regex |
| `--slim` | | Strip comments, collapse blanks |
| `--map` | `-m` | Signatures and definitions only |
| `--depth` | `-d` | Limit tree depth |

## Docs

Detailed usage for each command:

- [Context](https://github.com/AndrewPBerg/supp/blob/main/docs/context.md) — file extraction
- [Diff](https://github.com/AndrewPBerg/supp/blob/main/docs/diff.md) — git diffs and modes
- [Tree](https://github.com/AndrewPBerg/supp/blob/main/docs/tree.md) — directory tree
- [Config](https://github.com/AndrewPBerg/supp/blob/main/docs/config.md) — configuration

## Token estimation

supp shows an approximate token count for all output (`≈ ~N tokens`). This uses a fast heuristic — `bytes / 3.5` — rather than running a full BPE tokenizer. For mixed code, this is typically accurate within ~10% of the true cl100k count. The tradeoff is speed: estimation is instant, while tokenization would add hundreds of milliseconds.

## Symbol cache

`supp sym`, `supp why`, and `supp <paths>` build a symbol index using tree-sitter. The index is cached per project at `.git/supp/sym-cache` (or `/tmp/supp-sym-<hash>` for non-git directories).

On subsequent runs, supp checks file mtimes and sizes — only changed files are re-parsed. If nothing changed, the cached index is used as-is.

To force a full rebuild, delete the cache:

```sh
supp clean-cache          # current directory
supp clean-cache ~/myproj # specific project
```

The cache is scoped to each project root (the directory you pass to supp). Different projects maintain independent caches.

## Shell completions

```sh
# Bash
echo 'eval "$(supp completions bash)"' >> ~/.bashrc

# Zsh
echo 'eval "$(supp completions zsh)"' >> ~/.zshrc

# Fish
supp completions fish > ~/.config/fish/completions/supp.fish
```

## Managing supp

```sh
supp version    # check version (auto-checks for updates)
supp update     # update to latest release
supp uninstall  # remove from system
```
