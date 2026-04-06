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
| `supp <paths>` | Bundle files into structured context with token estimate |
| `supp diff` | Git diff with file tree, line counts, and full patches |
| `supp tree` | Project layout with git status markers |
| `supp sym <query>` | Find functions, types, and constants by name |
| `supp why <symbol>` | Explain a symbol — definition, call sites, and dependencies |
| `supp pick` | Interactive file picker (requires fzf) |
| `supp perf [mode]` | Set or check the global performance mode |
| `supp clean-cache` | Delete the symbol cache for a project |

> **NOTE:** `supp pick` requires [fzf](https://github.com/junegunn/fzf) to be installed and available on your `PATH`. Install it via your package manager (e.g. `brew install fzf`, `winget install fzf`, `pacman -S fzf`, `xbps-install fzf`) before using this command.
| `supp completions <shell>` | Generate shell completions (bash, zsh, fish) |

## Useful flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
| `--json` | | Output as JSON (machine-readable) |
| `--regex` | `-r` | Filter paths by regex |
| `--slim` | | Reduce noise: strip comments, collapse blanks |
| `--map` | `-m` | Outline mode: signatures, types, and API surface only |
| `--depth` | `-d` | Limit tree depth |
| `--perf` | `-p` | Override performance mode for this command |

## Docs

Detailed usage for each command:

- [Context](https://github.com/AndrewPBerg/supp/blob/main/docs/context.md) — bundling files into LLM context
- [Diff](https://github.com/AndrewPBerg/supp/blob/main/docs/diff.md) — git diffs and comparison modes
- [Sym](https://github.com/AndrewPBerg/supp/blob/main/docs/sym.md) — finding symbols by name
- [Why](https://github.com/AndrewPBerg/supp/blob/main/docs/why.md) — deep-diving a symbol
- [Tree](https://github.com/AndrewPBerg/supp/blob/main/docs/tree.md) — directory tree
- [Examples](https://github.com/AndrewPBerg/supp/blob/main/docs/examples.md) — workflows and multi-language demos
- [Config](https://github.com/AndrewPBerg/supp/blob/main/docs/config.md) — configuration
- [Performance](https://github.com/AndrewPBerg/supp/blob/main/docs/perf.md) — performance modes for large codebases

## Claude Code integration

supp ships with [Claude Code skills](https://docs.anthropic.com/en/docs/claude-code/skills) that let Claude use supp directly. Once supp is installed, these slash commands are available in any Claude Code session inside a project:

| Slash command | What it does |
|---------------|-------------|
| `/ctx <paths>` | Read files with project tree and token estimate |
| `/diff` | Review git changes with structured patches |
| `/tree` | See project layout with git status |
| `/sym <query>` | Find a symbol by name |
| `/why <symbol>` | Explain a symbol — definition, call sites, dependencies |

### Suggested prompts

These work well as starting points for Claude Code conversations:

```
# Orient yourself in an unfamiliar project
"Use /tree and /ctx --map . to map out this codebase, then summarize the architecture."

# Understand a specific function before changing it
"Use /why handle_request to explain how it works, then suggest how to add rate limiting."

# Review your own changes
"Use /diff to review my changes and suggest improvements."

# Explore a domain concept across languages
"Use /sym User to find all User-related types, then /why the most important one."
```

## Performance modes

On large codebases, supp can use significant CPU and memory. Set a global mode with `supp perf` or override per-command with `-p`:

```sh
supp perf lite                      # set globally (persisted)
supp perf                           # check current mode
supp -p full sym handler            # override for one command
```

| Mode | Threads | Best for |
|------|---------|----------|
| `full` | all cores | Small-to-medium projects (default) |
| `balanced` | half cores | Large projects (20k-100k files) |
| `lite` | 2 | Monorepos (100k+ files), constrained environments |

See [Performance Modes](https://github.com/AndrewPBerg/supp/blob/main/docs/perf.md) for technical details on what each mode tunes.

## Token estimation

supp shows an approximate token count for all output (`≈ ~N tokens`). This uses a fast heuristic — `bytes / 3.0` — rather than running a full BPE tokenizer. The conservative divisor means estimates lean slightly high, providing a safety buffer for whitespace-heavy or indentation-heavy code that tokenizes at worse ratios. The tradeoff is speed: estimation is instant, while tokenization would add hundreds of milliseconds.

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
