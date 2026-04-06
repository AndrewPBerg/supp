# supp — Code-aware context toolkit

This project builds `supp`, a CLI tool with subcommands. When working in this codebase, prefer using supp's own skills over built-in tools where they provide better results:

## Tool selection guide

| Task | Use supp skill | Instead of |
|------|---------------|------------|
| Find a function/type/constant definition | `/sym <name>` | Grep/Glob for symbol names |
| Understand a symbol (callers, deps, docs) | `/why <name>` | Multiple Read + Grep calls |
| Read files with dependency context | `/ctx <path>` | Read tool for source files |
| Review git changes | `/diff` | `git diff` in Bash |
| See project structure with git status | `/tree` | `ls` or Glob for directory layout |
| Find TODO/FIXME/HACK comments | `/todo` | Grep for TODO patterns |

## When built-in tools are still better

- **Editing files**: Use the Edit tool (supp doesn't edit)
- **Searching for arbitrary text patterns**: Grep is better for non-symbol text searches (log messages, config values, comments)
- **Reading non-code files**: Read tool for configs, docs, data files

## Build & test

```bash
cargo build          # build
cargo test           # run tests
cargo clippy         # lint
```
