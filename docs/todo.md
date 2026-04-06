# supp todo

Find TODO, FIXME, HACK, and XXX comments across your codebase. Uses tree-sitter to scan only real comments (not string literals or code), with optional git blame and surrounding context. Alias: `supp t`.

## Usage

```
supp [-n] todo [PATH] [OPTIONS]
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Show results only, skip clipboard copy |
| `--no-color` | | Disable colored output |
| `--json` | | Output as JSON (machine-readable) |
| `--regex` | `-r` | Regex pattern to filter file paths |

## Options

| Flag | Short | Description |
|------|-------|-------------|
| `--tags` | `-t` | Filter by tag types, comma-separated (TODO,FIXME,HACK,XXX) |
| `--blame` | `-B` | Include git blame info (author, date) per item |
| `--context` | `-C` | Number of context lines around each match (default: 0) |

## Examples

```bash
# Scan current directory for all tags
supp todo

# Scan a specific directory
supp todo src/

# Only FIXMEs and HACKs
supp todo -t FIXME,HACK

# Include blame info
supp todo -B

# Show 2 lines of surrounding context
supp todo -C 2

# Combine: FIXMEs in Rust files with blame
supp todo -t FIXME -r '\.rs$' -B

# JSON output for tooling
supp todo --json
```

## Example output

```
  supp todo  12 items in 87 files
  ────────────────────────────────────────

TODO
  src/cli.rs:42 — add validation for edge case
  src/main.rs:18 — refactor startup sequence

FIXME
  src/compress/mod.rs:210 — handle multi-byte edge case
  src/why.rs:55 — resolve circular deps

HACK
  src/styles.rs:30 — workaround for terminal width

  ✓ Copied to clipboard (1.2 KB)
  ≈ ~312 tokens (est.)
```

With `--blame`:

```
TODO
  src/cli.rs:42 — add validation  (Alice, 2025-11-14)
  src/main.rs:18 — refactor startup  (Bob, 2025-10-03)
```

## How it works

For files with tree-sitter support (Rust, Python, TS, JS, Go, C, C++, Java, etc.), supp parses the AST and only checks comment nodes — tags inside strings or variable names are ignored. For unsupported file types, it falls back to line-by-line regex matching.

Results are grouped by tag (TODO → FIXME → HACK → XXX), then sorted by file and line number. Files are scanned in parallel.
