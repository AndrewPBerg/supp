# supp tests-for

Find the likely validation surface for a file, symbol, or current git changes. This bridges code context to test context: "if I change this, what should I run?"

Alias: `supp test-map`.

## Usage

```
supp [-n] tests-for [TARGET] [OPTIONS]
supp [-n] test-map [TARGET] [OPTIONS]
```

## Options

| Flag | Description |
|------|-------------|
| `--diff` | Infer targets from `git status --porcelain` changed files |

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
| `--no-color` | | Disable colored output |
| `--json` | `-j` | Output JSON |

## What it reports

- Resolved target file when a symbol name is provided
- Likely test files based on naming and path heuristics
- Focused validation commands
- Warnings, such as test files outside configured pytest `testpaths`

## Examples

```bash
# Tests likely covering a file
supp -n tests-for src/auth/session.py

# Resolve a symbol first, then infer tests
supp -n tests-for create_session

# Infer tests for current changed files
supp -n tests-for --diff
```

## Notes

This command is intentionally heuristic. It should give a strong starting point, not pretend to be a full coverage engine. If it cannot find a direct test file, it still prints a focused command based on project configuration.
