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
| `--json` | | Output as JSON (machine-readable) |
| `--regex` | `-r` | Regex pattern to filter file paths |
| `--slim` | | Strip comments and collapse blank lines |
| `--map` | `-m` | Extract only signatures and definitions (codemap) |

## Options

| Flag | Short | Description |
|------|-------|-------------|
| `--untracked` | `-u` | Untracked (new) files |
| `--tracked` | `-t` | Unstaged changes to tracked files |
| `--staged` | `-s` | Staged changes vs HEAD |
| `--local` | `-l` | All local changes vs self branch remote |
| `--all` | `-a` | All branch changes vs remote default main (default) |
| `--branch <BRANCH>` | `-b` | Branch to compare to (used with `-a`) |
| `--unified` | `-U` | Number of context lines in unified diff output (default: 3, configurable) |


## Modes

| Command | Compares |
|---------|----------|
| `supp diff` | default branch ... current branch (fetches origin) |
| `supp diff -u` | untracked files |
| `supp diff -t` | index ... working directory (tracked only) |
| `supp diff -s` | HEAD ... index (staged only) |
| `supp diff -l` | origin/branch ... branch (local vs self remote) |
| `supp diff -a` | default branch ... current branch (explicit) |
| `supp diff -a -b develop` | develop ... current branch |

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
  ≈ ~8,432 tokens (est.)
```

An estimated token count (bytes / 3.5) is always shown. With `-n`, the clipboard step is skipped and the last line shows `– (27.6 KB, not copied)`.
