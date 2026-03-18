## Response Style

- **Always explain before showing code.** Concept first, implementation second.
- **Annotate every non-obvious line** in code examples with inline comments.
- **When there's an idiomatic Rust way**, show the naive version first, then refactor to idiomatic — don't just show the answer.
- **Call out ownership/borrowing decisions** explicitly, even when they seem obvious.
- **Use short summaries after code blocks** — one sentence on what to take away.
- **Flag when something is a simplification** — if a production version would differ, say so.

## Output Style (Local Override)

Treat all responses in this repo as if **Learning mode** is active:
- Pause and ask me to write small pieces of code for hands-on practice
- Explain implementation choices before showing solutions
- Prefer guided discovery over just giving the answer
# Supp — CLAUDE.md

> A CLI tool for extracting working tree context, git diffs, and file content for use in LLM prompts.
> Built in Rust, primarily as a **learning project** for a first-time Rustacean.

---

## Project Philosophy

This is a learning-first project. Correctness and idiomatic Rust matter more than shipping features fast. When there are multiple ways to do something, **prefer the approach that teaches the most** — even if it means doing it wrong first, then refactoring.

Performance experiments are welcome and encouraged. This is a good project to feel the difference between zero-cost abstractions and naive code.

---

## Learning Goals

- Understand Rust ownership, borrowing, and lifetimes in a real program
- Get comfortable with `Result`/`Option` error handling (no `.unwrap()` in final code)
- Learn how to structure a CLI binary with multiple subcommands
- Explore native OS speed-ups: `mmap`, parallel file walking, syscall-level I/O
- Build intuition for when to use `String` vs `&str`, `Vec` vs slices, etc.
- Practice writing idiomatic Rust — clippy-clean, formatted, documented
- Learn the full lifecycle of a published Rust binary: packaging, CI, cross-compilation

---

## Stack & Key Dependencies

| Crate | Purpose |
|---|---|
| `clap` (derive API) | Argument parsing and subcommands |
| `git2` | Libgit2 bindings — working tree, diffs, status |
| `ignore` | Respects `.gitignore`, fast parallel file walking |
| `walkdir` | Simple recursive file walking (A/B compare vs `ignore`) |
| `skim` or `fzf` (subprocess) | Fuzzy file picking — see Integrations |
| `colored` / `termcolor` | Terminal output coloring |
| `anyhow` | Ergonomic error handling for binaries |
| `thiserror` | Typed errors for library-style modules |
| `rayon` | Data parallelism for file processing experiments |
| `memmap2` | Memory-mapped file I/O experiments |
| `indicatif` | Progress bars for longer operations |

> **Note for Claude:** When suggesting new dependencies, always explain *why* that crate over alternatives and flag if it adds meaningful compile-time cost.

---

## Project Structure

```
supp/
├── src/
│   ├── main.rs          # Entry point, clap CLI definition
│   ├── cli.rs           # Subcommand structs (derive API)
│   ├── git.rs           # Git operations via git2
│   ├── walker.rs        # File discovery strategies
│   ├── output.rs        # Formatting and rendering context
│   ├── picker.rs        # fzf / skim integration
│   └── bench.rs         # A/B/C experiment harness (feature-gated)
├── extension/
│   ├── Cargo.toml       # [lib] crate-type = ["cdylib"] for WASM
│   ├── src/
│   │   └── lib.rs       # Zed extension entry point
│   └── extension.toml   # Zed extension manifest
├── benches/             # Criterion benchmarks
├── tests/               # Integration tests
├── packaging/
│   ├── aur/             # PKGBUILD and .SRCINFO for AUR
│   └── xbps/            # void-packages template
├── .github/
│   └── workflows/
│       ├── ci.yml       # Test + clippy on every push
│       └── release.yml  # Cross-compile + publish on tag
├── CLAUDE.md
└── Cargo.toml
```

---

## CLI Design

```
supp [OPTIONS] [SUBCOMMAND]

Subcommands:
  diff        Show git diff (staged, unstaged, or both)
  tree        Print working tree (respects .gitignore by default)
  context     Combine tree + diff + selected file contents into a prompt block
  pick        Interactive file picker (fzf-backed), output paths to stdout

Global Options:
  --repo <PATH>       Path to git repo (default: cwd)
  --format <FORMAT>   Output format: plain | markdown | xml (default: markdown)
  --copy              Pipe output to clipboard (pbcopy / xclip / wl-copy)
```

> Keep subcommands composable. `supp context` should feel like a curated version of running the others together.

---

## A/B/C Experiment Guidelines

One of the main goals of this project is **benchmarking different Rust approaches** to the same problem. Good candidates:

- **File walking:** `walkdir` vs `ignore` vs raw `std::fs::read_dir` recursion
- **File reading:** `std::fs::read_to_string` vs `BufReader` vs `memmap2`
- **Parallelism:** sequential vs `rayon::par_iter` for multi-file context assembly
- **String building:** `String::push_str` vs `format!` vs `Vec<u8>` + `write!`

### How to structure experiments

1. Implement each variant as a standalone function with a clear name (`walk_walkdir`, `walk_ignore`, `walk_raw`)
2. Gate them behind a feature flag or a `--strategy` CLI flag so they're runtime-switchable
3. Write a Criterion benchmark in `benches/` that runs all variants against the same input
4. Add a comment block above each variant explaining what tradeoff it's testing

```rust
/// Strategy A: walkdir — simple, no gitignore awareness
/// Hypothesis: slower on large repos, simpler code
fn walk_walkdir(root: &Path) -> Vec<PathBuf> { ... }

/// Strategy B: ignore crate — respects .gitignore, parallel
/// Hypothesis: faster on large repos, more correct by default
fn walk_ignore(root: &Path) -> Vec<PathBuf> { ... }
```

---

## fzf / skim Integration

The `pick` subcommand shells out to `fzf` (falling back to `skim` if fzf isn't found). This keeps Rust code simple while leveraging a best-in-class fuzzy finder.

```rust
// Preferred pattern: pipe candidate list to fzf via stdin, collect selections from stdout
// Use std::process::Command — no need for a crate wrapper
```

- Check for `fzf` in `$PATH` at runtime; if missing, print a helpful install message
- Support multi-select (`fzf --multi`) so users can pick several files at once
- Pass `--preview 'cat {}'` as a default for file content preview
- The `context` subcommand should use `pick` internally when no files are specified

---

## Error Handling Rules

- **No `.unwrap()` or `.expect()` in non-test code.** Use `?` and `anyhow::Result` in `main` and subcommands.
- Use `thiserror` for any errors that live in a module boundary (e.g. `git.rs`, `walker.rs`)
- Propagate errors up cleanly; only convert to user-facing messages at the top level in `main.rs`
- If a git operation fails because we're not in a repo, emit a clear, friendly message — not a panic

---

## Code Style & Idioms

- Run `cargo fmt` and `cargo clippy -- -D warnings` before considering any code done
- Prefer iterator chains over `for` loops where it reads naturally
- Avoid unnecessary clones — if Claude suggests a clone, it should explain why ownership can't be borrowed instead
- Use `impl Trait` in function signatures where concrete types would be noisy
- Keep functions small and single-purpose; if a function needs a comment to explain *what* it does (not *why*), it should probably be split

---

## Packaging & Distribution

### Targets

Publish `supp` to two community repos for friends on Arch and Void Linux. Learn both packaging systems from first principles, not just copy-paste templates.

| Distro | Format | Repo |
|---|---|---|
| Arch Linux | PKGBUILD → AUR | `aur.archlinux.org/supp` |
| Void Linux | XBPS template | PR to `void-linux/void-packages` |

---

### Cross-compilation

Use `cross` (a Docker-based wrapper around cargo) to build release binaries for multiple targets from a single machine.

Key targets:
```
x86_64-unknown-linux-musl      # Statically linked — ideal for packaging
x86_64-unknown-linux-gnu       # Standard glibc Linux
aarch64-unknown-linux-gnu      # ARM64 (Raspberry Pi, ARM Arch installs)
```

> **Why musl for packages?** The musl target produces a fully static binary with no libc dependency. This makes PKGBUILD and XBPS templates much simpler — no need to declare `glibc` as a runtime dep or worry about version mismatches on the user's system. Always prefer the musl static target for release artifacts.

---

### AUR (Arch Linux)

`packaging/aur/` contains the `PKGBUILD` and `.SRCINFO`. The PKGBUILD should:
- Pull the pre-built musl binary from the GitHub release (not compile from source — keeps install fast)
- Use `pkgver` that matches the git tag exactly
- Be linted with `namcap` before publishing

Key AUR concepts to learn and understand:
- How `makepkg` uses `prepare()`, `build()`, and `package()` functions
- How to push to the AUR git remote (it's just a bare git repo)
- `updpkgsums` to refresh checksums after a release
- `aurpublish` as a helper tool for the publish flow

---

### XBPS (Void Linux)

The template lives at `packaging/xbps/template` and should:
- Use `build_style=fetch` with the pre-built musl binary
- Declare `short_desc`, `maintainer`, `license`, `homepage`, `distfiles`, `checksum`
- Be linted with `xlint` before submitting as a PR to `void-linux/void-packages`

Key XBPS concepts to learn:
- How `xbps-src` works and why Void builds everything from a central template tree
- Void packaging conventions around static binaries
- How to test the template locally with `./xbps-src pkg supp` before submitting

---

### GitHub Actions: Release Workflow

The release workflow (`.github/workflows/release.yml`) triggers on a version tag push (`v*.*.*`) and:

1. Runs `cargo test` and `cargo clippy` as a gate — no release if either fails
2. Uses `cross` to build binaries for all targets in a matrix
3. Creates a GitHub Release and uploads all binaries as assets
4. Updates `PKGBUILD` checksums and pushes to the AUR git remote
5. Prints the updated XBPS template (manual PR step for now)

```yaml
# Matrix build step structure
strategy:
  matrix:
    target:
      - x86_64-unknown-linux-musl
      - aarch64-unknown-linux-gnu
      - x86_64-unknown-linux-gnu
```

> **Note for Claude:** When writing GitHub Actions workflows, explain what each step does and why — especially around secrets, `GITHUB_TOKEN` permissions, and the `cross` invocation. Never write a workflow step without a comment.

---

## Zed Extension

### Goal

A Zed extension that sees the user's open tabs, combines their contents with git context (diff + tree), and assembles a ready-to-use LLM prompt block — without leaving the editor. This is the native Supp experience: trigger from the command palette, get your context block immediately.

### Architecture

Zed extensions are written in Rust and compiled to **WASM** (`wasm32-wasi` target via `zed_extension_api`). They run sandboxed, which means:

- No direct filesystem access — must use Zed's workspace APIs to read buffer content
- No arbitrary subprocess spawning from within WASM
- Communication with the editor happens through the `zed_extension_api` crate

**Strategy: thin extension, fat CLI.** The extension is a coordinator. It reads open buffers via Zed's API, then delegates to the already-installed `supp` binary on `$PATH` for git operations (diff, tree). This avoids reimplementing git logic in WASM and keeps the extension small.

The extension lives in its own crate:

```
extension/
├── Cargo.toml        # crate-type = ["cdylib"], target wasm32-wasi
├── src/
│   └── lib.rs        # implements zed_extension_api::Extension trait
└── extension.toml    # manifest: name, version, commands
```

### What the extension does

1. Registers a **"Supp: Build Context"** command in the Zed command palette
2. Enumerates all open buffer paths and their contents via `zed_extension_api`
3. Calls the `supp` CLI binary via `zed_extension_api::process::Command` to get git diff + tree output
4. Assembles the final context block (open files + git output) in the same format as `supp context`
5. Writes the result to a new scratch buffer or copies to clipboard

### Key learning concepts

- Compiling Rust to WASM (`wasm32-wasi` target, `cdylib` crate type)
- Capability-based sandboxing — understanding what the WASM runtime can and can't touch
- Designing an integration layer that delegates to a CLI rather than reimplementing logic
- Zed's extension manifest format and how commands are registered

> **Note for Claude:** When helping with the Zed extension, always note which `zed_extension_api` APIs are being used and whether they're stable or experimental. The Zed extension API is still evolving — flag anything that might break across Zed versions.

---

## What to Ask Claude For

Good uses of Claude on this project:

- **"Explain this borrow checker error"** — always ask for the *why*, not just the fix
- **"What's the idiomatic way to do X in Rust?"** — there's usually a clippy lint or std method you don't know about yet
- **"Write both a naive and an idiomatic version"** — great for learning the gap
- **"Add a Criterion benchmark for this function"** — benchmarks are first-class here
- **"Write the GitHub Actions release workflow step by step"** — explain every line
- **"What does this PKGBUILD line actually do?"** — packaging is full of tribal knowledge, always ask
- **"Show me what the WASM sandbox can and can't access"** — important before writing any extension code

### Things Claude must always do in this repo

- Explain *why* before showing *how*
- If suggesting `.clone()`, justify it — can we borrow instead?
- If suggesting a new crate, compare it to at least one alternative
- Flag when something is idiomatic Rust vs a shortcut taken for simplicity
- When writing CI or packaging config, comment every non-obvious line
- When the Zed API is involved, flag stability and version sensitivity
