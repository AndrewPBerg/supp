# Shell Completions & fzf Integration

## Shell Completions

Generate shell completion scripts with the `completions` subcommand, then install for your shell:

### Bash

```
supp completions bash > ~/.local/share/bash-completion/completions/supp
```

### Zsh

```
supp completions zsh > ~/.zfunc/_supp
```

Add to your `~/.zshrc` (if not already present):

```zsh
fpath+=~/.zfunc
autoload -Uz compinit && compinit
```

### Fish

```
supp completions fish > ~/.config/fish/completions/supp.fish
```

## fzf Integration

The `pick` subcommand (alias `p`) launches [fzf](https://github.com/junegunn/fzf) for interactive file selection, generates context from the selected files, and copies it to the clipboard.

### Usage

```
supp pick [PATH] [OPTIONS]
```

### Options

| Flag | Short | Description |
|------|-------|-------------|
| `--single` | `-1` | Select a single file (skips confirmation and accumulation) |
| `--regex` | `-r` | Regex pattern to pre-filter the file list |

### Behavior

By default, `pick` opens fzf in multi-select mode. After selecting files, you enter an interactive loop where you can confirm, add more files, or clear. Once confirmed, context is generated and copied to the clipboard.

With `--single` (`-1`), fzf opens in single-select mode and immediately generates context — no confirmation step.

### Examples

```bash
# Interactive multi-select with confirm loop
supp pick

# Single file, no confirmation
supp pick -1

# Pick from a specific directory
supp pick src/

# Pre-filter to only Rust files
supp -r '\.rs$' pick
```

### Keybindings

| Key | Action |
|-----|--------|
| `ctrl-space` | Toggle selection on current item |
| `enter` | Toggle all visible (filtered) items — press again to deselect all |
| `tab` | Confirm selection and proceed |
| `esc` | Cancel and exit |

### fzf search syntax

fzf supports powerful inline search patterns. Combine them with spaces (AND logic):

| Pattern | Meaning |
|---------|---------|
| `agent` | Fuzzy match "agent" |
| `'agent` | Exact match "agent" |
| `^src/` | Starts with "src/" |
| `.py$` | Ends with ".py" |
| `!migration` | Does NOT contain "migration" |
| `agent !migration` | Contains "agent" AND NOT "migration" |
| `agent !migration !test` | Contains "agent", excludes "migration" and "test" |
| `'agents/ .py$` | Exact "agents/" AND ends with ".py" |

**Tip:** Type a query to filter, then press `enter` to select all visible matches at once.

### Requirements

`fzf` must be installed. If it is not found, `supp pick` will print install instructions.
