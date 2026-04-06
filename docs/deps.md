# supp deps

Visualize file-level import/dependency relationships across your project. Shows which files import which, with optional focus on a single file, reverse lookups, depth limiting, and DOT graph output. Alias: `supp d`.

## Usage

```
supp [-n] deps [PATH] [OPTIONS]
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
| `--reverse` | `-R` | Show reverse dependencies (what depends on the target) |
| `--depth` | `-d` | Maximum traversal depth |
| `--dot` | | Output DOT format for Graphviz |

## Examples

```bash
# Whole-project dependency graph
supp deps

# Focus on a single file's dependencies
supp deps src/main.rs

# What depends on this file? (reverse)
supp deps src/compress/mod.rs -R

# Limit traversal to 2 hops
supp deps src/main.rs -d 2

# Generate Graphviz DOT output
supp deps --dot > deps.dot
dot -Tpng deps.dot -o deps.png

# Focus + reverse + depth
supp deps src/ctx.rs -R -d 1

# Filter to Rust files only
supp deps -r '\.rs$'

# JSON output for tooling
supp deps --json
```

## Example output

### Whole-project mode

```
  supp deps  14 files, 22 edges
  ────────────────────────────────────────

src/cli.rs → src/config.rs, src/compress/mod.rs
src/ctx.rs → src/compress/mod.rs, src/pick.rs, src/styles.rs
src/main.rs → src/cli.rs, src/ctx.rs, src/deps.rs, src/todo.rs
src/why.rs → src/compress/mod.rs, src/pick.rs
```

### Focused mode (forward)

```
src/main.rs (dependencies):
├── src/cli.rs
│   └── src/config.rs
├── src/ctx.rs
│   ├── src/compress/mod.rs
│   └── src/pick.rs
├── src/deps.rs
│   ├── src/compress/mod.rs
│   ├── src/pick.rs
│   └── src/why.rs
└── src/todo.rs
    └── src/compress/mod.rs
```

### Focused mode (reverse)

```
src/compress/mod.rs (dependents):
├── src/ctx.rs
├── src/deps.rs
├── src/todo.rs
└── src/why.rs
```

## How it works

supp builds a file-level dependency graph by parsing imports/requires across the project using tree-sitter. Module paths are resolved to actual project files — external packages are excluded.

Supported import resolution:
- **Rust**: `crate::`, `super::` module paths → `src/*.rs` or `src/*/mod.rs`
- **Python**: relative (`.foo`) and absolute (`foo.bar`) imports → `*.py` or `*/__init__.py`
- **JS/TS**: relative paths (`./`, `../`) with extension probing (`.ts`, `.tsx`, `.js`, `.jsx`, `index.*`)
- **C/C++**: local `#include "..."` headers

When focused on a target file, supp performs a BFS traversal from that file through the graph. With `--reverse`, the graph is inverted first so edges point from dependents to dependencies.
