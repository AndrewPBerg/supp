# supp diff

Compare changes in a git repository and copy the result to the clipboard.

## Usage

```
supp [-n] diff [PATH] [OPTIONS]
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Show stats only, skip clipboard copy |
| `--no-color` | | Disable colored output |
| `--regex` | `-r` | Regex pattern to filter file paths |
| `--slim` | `-S` | Strip comments and collapse blank lines |
| `--map` | `-M` | Extract only signatures and definitions (codemap) |

## Options

| Flag | Short | Description |
|------|-------|-------------|
| `--cached` | `-c` | Staged files vs HEAD |
| `--untracked` | `-u` | Untracked (new) files |
| `--local` | `-l` | Unstaged working directory changes |
| `--branch <BRANCH>` | `-b` | Base branch to compare against |
| `--all` | `-a` | All local changes (untracked + staged + unstaged) |
| `--self-branch` | `-s` | Unpushed commits vs `origin/<current-branch>` |
| `--unified` | `-U` | Number of context lines in unified diff output (default: 3, configurable) |
| `--filter` | `-f` | Glob pattern to filter files (e.g. `*.rs`) |

Defaults for `--no-copy`, `--unified`, and the max untracked file size can be set in [`~/.supp/config.toml`](config.md).

## Modes

| Command | Compares |
|---------|----------|
| `supp diff` | default branch ... current branch (fetches origin) |
| `supp diff -c` | HEAD ... index |
| `supp diff -l` | index ... working directory |
| `supp diff -u` | untracked files |
| `supp diff -b develop` | develop ... current branch |
| `supp diff -a` | all local changes combined |
| `supp diff -s` | origin/branch ... branch (unpushed commits) |

## Example output

```
  supp diff  main ... diff-functionality
  ────────────────────────────────────────

├── Cargo.lock    modified   +243  -0
├── Cargo.toml    modified     +2  -1
└── src/
    ├── cli.rs    modified     +8  -2
    ├── git.rs    modified   +120 -45
    └── main.rs   modified    +98 -20

  5 files  (5 modified)   +471 -68

  ✓ Copied to clipboard (27.6 KB)
  ≈ ~8,432 tokens (cl100k est.)
```

An estimated token count (using `cl100k_base`) is always shown. With `-n`, the clipboard step is skipped and the last line shows `– (27.6 KB, not copied)`.
