# Configuration

supp reads an optional config file at `~/.supp/config.toml`. Every key is optional — missing keys use hardcoded defaults. CLI flags always override config values.

## Precedence

```
CLI flag  >  config.toml  >  hardcoded default
```

## File Format

```toml
# All keys optional. Missing = hardcoded default.

[global]
no_copy = false        # skip clipboard by default
no_color = false       # disable colored output
depth = 2              # tree depth in context header
mode = "full"          # "full" | "slim" | "map"

[diff]
context_lines = 3      # unified diff context lines (-U)

[pick]
preview_lines = 100    # lines shown in fzf preview

[limits]
max_untracked_file_size_mb = 10   # skip untracked files larger than this
```

## Sections

### `[global]`

| Key | Type | Default | CLI override |
|-----|------|---------|--------------|
| `no_copy` | bool | `false` | `-n` / `--no-copy` |
| `no_color` | bool | `false` | `--no-color` |
| `depth` | integer | `2` | `-d` / `--depth` |
| `mode` | string | `"full"` | `-S` / `--slim`, `-M` / `--map` |

`mode` accepts `"full"`, `"slim"`, or `"map"`. CLI flags `-S` and `-M` always win.

### `[diff]`

| Key | Type | Default | CLI override |
|-----|------|---------|--------------|
| `context_lines` | integer | `3` | `-U` / `--unified` |

### `[pick]`

| Key | Type | Default |
|-----|------|---------|
| `preview_lines` | integer | `100` |

Controls how many lines `fzf` shows in the file preview pane.

### `[limits]`

| Key | Type | Default |
|-----|------|---------|
| `max_untracked_file_size_mb` | integer | `10` |

Untracked files larger than this limit are skipped in `supp diff -u` and `supp diff -a` to avoid OOM.

## Behavior

- If `~/.supp/config.toml` does not exist, supp works exactly as before (all hardcoded defaults).
- If the file exists but contains invalid TOML, a warning is printed to stderr and defaults are used.
- Boolean config values (`no_copy`, `no_color`) are OR'd with CLI flags — setting either one enables the behavior.

## Examples

Always skip clipboard copy and use depth 4:

```toml
[global]
no_copy = true
depth = 4
```

Use slim mode by default with 5 context lines in diffs:

```toml
[global]
mode = "slim"

[diff]
context_lines = 5
```
