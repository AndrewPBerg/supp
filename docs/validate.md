# supp validate

Suggest or run the narrowest useful validation command for a target, changed files, or the whole project.

The default behavior is safe: print commands only. Use `--run` to execute the first suggested command.

## Usage

```
supp [-n] validate [TARGET] [OPTIONS]
```

## Options

| Flag | Description |
|------|-------------|
| `--changed` | Infer validation commands from changed files in `git status` |
| `--fast` | Prefer the fastest useful project-level validation |
| `--run` | Execute the first suggested command |

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
| `--no-color` | | Disable colored output |
| `--json` | `-j` | Output JSON |

## Examples

```bash
# Suggest project-level validation
supp -n validate

# Suggest validation for one file
supp -n validate src/main.rs

# Suggest validation for changed files
supp -n validate --changed

# Run the first suggested command
supp -n validate --changed --run
```

## What it can infer

- Rust/Cargo: `cargo test`
- Go: `go test ./...` or a package-level command
- Python/pytest: `pytest`, `uv run pytest`, or a target path
- JavaScript/TypeScript: package-manager test scripts when discoverable

## Philosophy

`validate` is a router, not a test framework. It should pick a sane next command and keep the agent from guessing wildly. When confidence is low, it prints a warning instead of inventing certainty.
