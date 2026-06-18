# supp review

Build a reviewer-focused packet from the current diff. `supp review` is a beefier, review-oriented sibling of `supp diff`: it keeps the patch, then adds likely tests, focused validation commands, warnings, and docs/comment context that may matter for review.

## Usage

```
supp [-n] review [PATH] [OPTIONS]
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
| `--no-color` | | Disable colored output |
| `--json` | `-j` | Output JSON |
| `--regex` | `-r` | Filter changed file paths by regex |

## Options

`review` accepts the same diff mode flags as `diff`:

| Flag | Short | Description |
|------|-------|-------------|
| `--untracked` | `-u` | Untracked files only |
| `--tracked` | `-t` | Unstaged changes to tracked files |
| `--staged` | `-s` | Staged changes only |
| `--local` | `-l` | All local changes vs self branch remote |
| `--all` | `-a` | All branch changes vs remote default main |
| `--branch <BRANCH>` | `-b` | Branch to compare to (used with `-a`) |
| `--unified <N>` | `-U` | Number of context lines in patch output |

## What it includes

- changed files with status and line counts
- likely related test files from `supp tests --diff`
- focused validation commands
- warnings, including tests outside configured discovery when detected
- related docs/docstrings/comment snippets from `supp docs`-style search
- full patch text for review

## Examples

```bash
# Review tracked unstaged changes
supp -n review -t

# Review staged changes before commit
supp -n review -s

# Review current branch against default remote
supp -n review

# Review only Rust files
supp -n review -t -r '\.rs$'
```

## When to use

Use `supp review` for code review, PR prep, or before asking an agent to critique a diff. Use `supp diff` when you only need the patch.
