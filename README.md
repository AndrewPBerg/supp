# supp

Structured code context for LLMs. Extracts files, diffs, symbols, and trees from your codebase and copies them to the clipboard — ready to paste into any chat.

[![CI](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml/badge.svg)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/AndrewPBerg/supp/branch/main/graph/badge.svg)](https://codecov.io/gh/AndrewPBerg/supp)

## Install

```sh
# From crates.io
cargo install supp

# Or via install script
curl -fsSL https://raw.githubusercontent.com/AndrewPBerg/supp/main/install.sh | bash
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
| `supp <paths>` | Extract file contents with token count |
| `supp diff` | Structured diff between branches |
| `supp tree` | Directory tree with git status markers |
| `supp sym <query>` | Search symbols with PageRank ranking |
| `supp why <symbol>` | Full context for a symbol |
| `supp pick` | Interactive file picker (requires fzf) |
| `supp mcp` | Start as an MCP server |

## Useful flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
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

## Managing supp

```sh
supp version    # check version (auto-checks for updates)
supp update     # update to latest release
supp uninstall  # remove from system
```
