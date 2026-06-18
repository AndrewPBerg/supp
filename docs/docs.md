# supp docs

Search documentation, doc comments, docstrings, and inline comments for context. This is a lightweight context-finding command for agents: it answers "what prose explains this concept?" without dumping source files.

## Usage

```
supp [-n] docs [QUERY...] [OPTIONS]
```

## Options

| Flag | Short | Description |
|------|-------|-------------|
| `--limit <N>` | `-l` | Maximum matches to show (default: 20) |

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Print only, skip clipboard |
| `--no-color` | | Disable colored output |
| `--json` | `-j` | Output JSON |

## What it searches

- Markdown and text docs (`README.md`, `AGENTS.md`, `CLAUDE.md`, `docs/*.md`, etc.)
- Rust, Go, Python, TypeScript/JavaScript, Java, C, and C++ comments
- Rust doc comments (`///`, `//!`)
- JSDoc/Javadoc/block comments (`/** ... */`)
- Python docstrings and comments
- Inline comments when they match the query

## Examples

```bash
# Find prose around auth/session concepts
supp -n docs auth session

# Find local agent/project conventions
supp -n docs AGENTS conventions

# More matches
supp -n docs pytest testpaths -l 40

# JSON for editor/agent integrations
supp -n docs selector contract --json
```

## When to use

Use `supp docs` when source alone is not enough and you want the repository's own prose: design notes, local conventions, stale path hints, docstrings, TODO-like explanation, or reviewer context.

It is not a replacement for `rg` when you need an exact literal string. It is a context search over human-written explanations.
