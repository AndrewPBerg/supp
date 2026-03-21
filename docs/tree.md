# supp tree

Display a directory tree with git status indicators and copy the result to the clipboard.

## Usage

```
supp [-n] tree [PATH] [OPTIONS]
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Show tree only, skip clipboard copy |
| `--no-color` | | Disable colored output |
| `--regex` | `-r` | Regex pattern to filter file paths |

## Options

| Flag | Short | Description |
|------|-------|-------------|
| `--depth` | `-d` | Maximum depth to display |
| `--no-git` | | Disable git status indicators |

## Example output

```
  supp tree  .
  ────────────────────────────────────────

  ./
  ├── Cargo.lock [M]
  ├── Cargo.toml [M]
  ├── docs/
  │   └── diff.md
  └── src/
      ├── cli.rs [M]
      ├── git.rs [M]
      ├── main.rs [M]
      ├── styles.rs [A]
      └── tree.rs [A]

  2 directories, 10 files (5 modified, 2 added)

  ✓ Copied to clipboard (268 B)
  ≈ ~81 tokens (cl100k est.)
```

An estimated token count (using `cl100k_base`) is always shown. With `-n`, the clipboard step is skipped.
