# supp commands

List discovered build, test, lint, eval, and benchmark commands from project configuration. This keeps agents from guessing `npm test`, `pytest`, `cargo test`, or custom Poe/script names.

## Usage

```
supp [-n] commands
supp [-n] cmds
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
| `--no-color` | | Disable colored output |
| `--json` | `-j` | Output JSON |

## What it reports

Each command includes:

- command string
- inferred purpose (`test`, `check`, `lint`, `eval`, `benchmark`, `other`)
- source, such as `Cargo.toml`, `go.mod`, `pyproject.toml`, Poe tasks, or `package.json` scripts

## Examples

```bash
# Human-readable command list
supp -n commands

# Alias
supp -n cmds

# JSON for integrations
supp -n commands --json
```

## When to use

Use `supp commands` before validation or when entering an unfamiliar repo. It is intentionally narrower than `supp config`: ask only for command surface when that is all you need.
