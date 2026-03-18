# Supp — Pickup Notes

## Where we left off

Completed a working `supp diff` command that:
- Opens the local git repo via `git2`
- Gets the unstaged diff (working dir vs index)
- Prints it line by line, skipping non-UTF8 bytes

## Current file structure

```
src/
  main.rs    — CLI wiring, match routing, anyhow::Result on main
  cli.rs     — Cli struct + Commands enum (Diff, Tree as struct variants)
  git.rs     — get_diff(repo_path: &str) -> Result<()>
```

## What works

- `supp diff` — prints unstaged git diff
- `supp diff --repo <path>` — diff for a specific repo path
- `supp tree` — prints "tree!" (stub only, not implemented)
- `--repo` is scoped per-subcommand, not global

## Next feature: `supp tree`

Implement the `tree` subcommand using the `ignore` crate (already in Cargo.toml).

Plan:
1. Create `src/walker.rs`
2. Write `walk_ignore(root: &Path) -> Vec<PathBuf>` using the `ignore` crate
3. Print each path to stdout
4. Wire it into the `Tree` match arm in `main.rs`

The `ignore` crate respects `.gitignore` automatically and walks in parallel.
This is also the first A/B experiment opportunity — compare `ignore` vs `walkdir`.

## Concepts covered this session

- `mod` vs `use` — declare module vs bring into scope
- `pub` — everything private by default, opt in to share
- `Option<T>` — Rust's null safety (`Some` / `None`)
- `Result<T>` and `?` — error propagation without exceptions
- `match` and exhaustive pattern matching
- Struct variants in enums (`Diff { repo: Option<String> }`)
- `#[arg(long)]` / `#[arg(short)]` — clap flag styles
- `&str` vs `String` — borrowed vs owned strings
- `if let Ok(s)` — pattern match on Result without unwrap
- `::` vs `.` — type namespace vs instance method

## Things to explore next

- `supp tree` with `ignore` crate
- A/B benchmark: `ignore` vs `walkdir` vs raw `std::fs::read_dir`
- `supp context` — combine diff + tree + file contents
- `supp pick` — fzf integration
- Colored output with `colored` crate
- `cargo clippy -- -D warnings` as a pre-commit habit
