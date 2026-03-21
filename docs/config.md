# Configuration

supp supports two optional config files named `supp.toml`:

| Level | Path |
|-------|------|
| Global | `~/.config/supp/supp.toml` (via `dirs::config_dir()`) |
| Local | `<git-repo-root>/supp.toml` |

Every key is optional — missing keys use hardcoded defaults. Local config overrides global config at the field level, and CLI flags always win.

## Precedence

```
CLI flag  >  local supp.toml  >  global supp.toml  >  hardcoded default
```

Fields are merged individually, not by file. If the global config sets `depth = 4` and the local config sets `no_copy = true`, the result has both `depth = 4` and `no_copy = true`.

## File Format

```toml
# All keys optional. Missing = inherited from lower-priority source.

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

- If neither config file exists, supp works exactly as before (all hardcoded defaults).
- If a file exists but contains invalid TOML, a warning is printed to stderr and that file is skipped.
- Boolean config values (`no_copy`, `no_color`) are OR'd with CLI flags — setting either one enables the behavior.
- Local config is only discovered when running inside a git repository. Outside a repo, only the global config is used.

## Examples

Global config — user-level defaults (`~/.config/supp/supp.toml`):

```toml
[global]
no_copy = true
depth = 4
```

Local config — per-repo overrides (`<repo-root>/supp.toml`):

```toml
[global]
mode = "slim"

[diff]
context_lines = 5
```

With both files, supp merges them: `no_copy = true`, `depth = 4`, `mode = "slim"`, `context_lines = 5`. CLI flags like `-d 1` override the merged result.
