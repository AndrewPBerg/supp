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

The `pick` subcommand launches [fzf](https://github.com/junegunn/fzf) for interactive file selection and prints the selected paths to stdout. This makes it composable with other commands.

### Usage

```
supp pick [PATH] [OPTIONS]
```

### Options

| Flag | Short | Description |
|------|-------|-------------|
| `--single` | `-s` | Select only one file (disables multi-select) |
| `--regex` | `-r` | Regex pattern to pre-filter the file list |

### Examples

```bash
# Pick files, then generate context
supp $(supp pick)

# Pick files, print context without clipboard
supp $(supp pick) -n

# Pick a single file, prints its path
supp pick --single

# Pick from a specific directory
supp pick src/

# Pre-filter to only Rust files
supp -r '\.rs$' pick
```

### Configuration

The number of preview lines shown in fzf (default: 100) can be changed in [`~/.supp/config.toml`](config.md):

```toml
[pick]
preview_lines = 50
```

### Requirements

`fzf` must be installed. If it is not found, `supp pick` will print install instructions.
