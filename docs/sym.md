# supp sym

Search symbols by name with PageRank-powered ranking. Alias: `supp s`.

## Usage

```
supp [-n] sym <query...>
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Show results only, skip clipboard copy |
| `--no-color` | | Disable colored output |
| `--json` | | Output as JSON (machine-readable) |

## How it works

`supp sym` builds a symbol index from the project using tree-sitter, then ranks matches using a PageRank-inspired algorithm based on cross-file references. The query is split into tokens and matched against symbol names.

Indexed symbol kinds: functions, methods, classes, structs, enums, interfaces, traits, type aliases, constants, and module-level assignments.

Supported languages: Rust, Python, TypeScript, TSX, JavaScript, Go, C, C++, Java, JSON (top-level keys), Markdown (headers).

## Examples

```bash
# Search for symbols matching "parse"
supp sym parse

# Multi-token query
supp sym git diff

# Use the alias
supp s config

# Print without copying
supp s -n handler
```

## Example output

```
  supp sym  parse
  ────────────────────────────────────────

  fn   parse_args         src/cli.rs:42        0.032
  fn   parse_config       src/config.rs:15     0.028
  fn   parse_imports      src/why.rs:310       0.019

  3 symbols

  ✓ Copied to clipboard (412 B)
  ≈ ~128 tokens (est.)
```

Results are sorted by relevance (PageRank score). The index is cached and rebuilt incrementally when files change.
