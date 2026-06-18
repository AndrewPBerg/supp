# supp project

Summarize the repository's identity in a small, agent-friendly packet. This is intended as a low-token startup/orientation command: enough to know the stack, test runner, entrypoints, and local instructions without dumping the whole repo.

## Usage

```
supp [-n] project [OPTIONS]
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
| `--no-color` | | Disable colored output |
| `--json` | `-j` | Output JSON |

## What it reports

- Languages detected from source files
- Framework hints from config files
- Package managers (`cargo`, `go`, `uv`, `npm`, etc.)
- Test runners (`cargo test`, `go test`, `pytest`, `vitest`, etc.)
- Configured test paths when discoverable; otherwise observed test directories
- Common commands from project config, such as Poe or package scripts
- Entrypoints like `src/main.rs`, `src/lib.rs`, and `main.go`
- Local instruction files such as `AGENTS.md`, `CLAUDE.md`, and `README.md`

## Examples

```bash
# Human-readable summary
supp -n project

# JSON for editor/agent integrations
supp -n project --json
```

## When to use

Use `supp project` at the start of a session or before planning validation. It is intentionally smaller than `supp tree` or full context generation: less is more.
