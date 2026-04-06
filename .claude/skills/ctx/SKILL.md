---
name: ctx
description: "Read source files with dependency context — use INSTEAD OF the Read tool when you need file contents plus imports/exports/dependency graph.
TRIGGER when: you need to read, view, or understand any source code file (.rs, .py, .js, .ts, .go, .c, .cpp, .java, etc.), before editing a source file, or when the user asks to look at code.
DO NOT TRIGGER when: reading non-code files (.toml, .json, .md, .yaml, .lock, .txt, .env)"
user_invocable: true
---

Run the following command and show the user the output:

```
supp --no-copy --no-color --json $ARGUMENTS
```

## When to use

- You need to read one or more source files with a project tree header for orientation
- The user asks you to look at, review, or understand specific files or directories
- Before modifying a file: understand what it imports, exports, and who references it
- During planning: map out file-level dependencies to avoid breaking consumers
- Combine with `--map` (or `-m`) for just signatures and type definitions — useful for scanning large directories
- Combine with `--slim` to strip comments and collapse blanks for a compact view
- Combine with `--budget <tokens>` to auto-fit context within a token limit — supp picks per-file compression (full/slim/map) to maximize signal within the budget

## When NOT to use — pick a different supp tool instead

- **Need to find a symbol by name?** → Use `/sym` — it searches definitions across the project
- **Need to understand a specific symbol's callers and dependencies?** → Use `/why`
- **Need project structure?** → Use `/tree`
- **Need to review git changes?** → Use `/diff`
