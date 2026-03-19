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

The `pick` subcommand launches [fzf](https://github.com/junegunn/fzf) for interactive file selection, then generates context from the selected files.

### Usage

```
supp [-n] pick [PATH] [OPTIONS]
```

### Options

| Flag | Short | Description |
|------|-------|-------------|
| `--single` | `-s` | Select only one file (disables multi-select) |

### Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Show context only, skip clipboard copy |
| `--regex` | `-r` | Regex pattern to pre-filter the file list |
| `--depth` | `-d` | Tree depth in context header (default: 2) |

### Examples

```bash
# Pick files from current directory
supp pick

# Pick a single file from a specific directory
supp pick src/ --single

# Pre-filter to only Rust files
supp -r '\.rs$' pick

# Pick without copying to clipboard
supp -n pick
```

### Requirements

`fzf` must be installed. If it is not found, `supp pick` will print install instructions.
