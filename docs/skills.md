# Claude Code Skills

supp ships with Claude Code skills — slash commands that inject supp output directly into your conversation.

## Available Skills

| Skill | Usage | Description |
|-------|-------|-------------|
| `diff` | `/project:diff [-c] [-a] ...` | Review git changes |
| `ctx` | `/project:ctx src/main.rs` | Single-file context analysis |
| `why` | `/project:why symbol_name` | Deep symbol explanation |
| `sym` | `/project:sym query` | Symbol search |
| `tree` | `/project:tree [-d 3] [path]` | Directory tree |

All flags from the CLI are supported — skills pass `$ARGUMENTS` directly to the corresponding `supp` command.

## Examples

```
/project:diff -c              # staged changes
/project:diff -a              # all local changes
/project:ctx src/lib.rs       # analyze a file
/project:why get_diff         # explain a symbol
/project:sym Config           # search for symbols
/project:tree -d 4 src/       # tree with depth 4
```

## Installation

### Project-scoped (automatic)

Skills are picked up automatically from `.claude/skills/` in the project root. If you cloned this repo, they're already available.

### Global

Copy the `.claude/skills/` directory to `~/.claude/skills/` to make them available in all projects:

```bash
cp -r .claude/skills/* ~/.claude/skills/
```

## Requirements

- `supp` must be in your `$PATH`
- `--no-copy` and `--no-color` are applied automatically (output goes to Claude, not your clipboard)
