# supp context

Bundle source files into structured, clipboard-ready context for an LLM. This is the default command — just pass paths, no subcommand needed. Output includes a project tree header, file contents, and a token estimate.

## Usage

```
supp [-n] <paths...> [OPTIONS]
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print context only, skip clipboard copy |
| `--no-color` | | Disable colored output |
| `--json` | | Output as JSON (machine-readable) |
| `--regex` | `-r` | Regex pattern to filter file paths |
| `--depth` | `-d` | Tree depth in context header (default: 2, configurable) |
| `--slim` | | Reduce noise: strip comments and collapse blank lines |
| `--map` | `-m` | Outline mode: extract only signatures, types, and API surface |
| `--budget` | | Token budget: auto-select per-file compression to fit within N tokens |

## Examples

```bash
# Single file
supp src/main.rs

# Multiple files and directories
supp src/ Cargo.toml

# Filter to only Rust files in a directory
supp src/ -r '\.rs$'

# Print without copying to clipboard
supp src/main.rs -n

# Pick files interactively, then generate context
supp pick
```

## Example output

```
  supp  3 files, 142 lines, 3.8 KB
  ────────────────────────────────────────

  ✓ Copied to clipboard (4.1 KB)
  ≈ ~1,024 tokens (est.)
  Done in 12ms
```

With `-n`, the clipboard step is skipped and shows `– (4.1 KB, not copied)` instead.

## Budget mode

Use `--budget <tokens>` to auto-fit context within a token limit. supp compresses each file at all three levels (full, slim, map), then packs the highest-fidelity versions that fit, prioritizing files by symbol importance (PageRank).

```bash
# Fit as much of src/ as possible into ~8,000 tokens
supp src/ --budget 8000

# Budget mode with a single file
supp src/main.rs --budget 4000
```

`--budget` conflicts with `--slim`, `--map`, and `--map-threshold` since it selects modes automatically.

### Example output

```
  supp  8 files, 320 lines, 10.2 KB
  ────────────────────────────────────────

  ✓ Copied to clipboard (14.8 KB)
  ↓ 120.5 KB → 10.2 KB (92% reduction)
  ≈ ~41,200 → ~5,100 tokens (est.)
  Done in 89ms

  budget ~8,000 target — 3 full, 2 slim, 3 map
```
