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

### Requirements

`fzf` must be installed. If it is not found, `supp pick` will print install instructions.
