---
name: tree
description: "Show directory layout with git status — ONLY for project structure or file location questions, not for understanding code or reviewing changes.
TRIGGER when: you need to see project structure, directory layout, what files exist, or orient yourself in an unfamiliar codebase.
DO NOT TRIGGER when: you need to read file contents (use /ctx), find symbols (use /sym), or review changes (use /diff)"
user_invocable: true
---

Run the following command and show the user the output:

```
supp tree --no-copy --no-color --json $ARGUMENTS
```

## When to use

- The user asks about project structure, "what's in this directory", or where files live
- You need to orient yourself in an unfamiliar project at the directory level
- You want to see which files are modified, added, or untracked at a glance
- Use `-d <N>` to limit depth for large projects

## When NOT to use — pick a different supp tool instead

- **Need to find a function/type/constant?** → Use `/sym` — it searches symbol definitions, not file paths
- **Need to understand what code does?** → Use `/why` — it shows definition, callers, and dependencies
- **Need to review git changes or diffs?** → Use `/diff` — it shows actual change content, not just modified markers
- **Need to read file contents?** → Use `/ctx` — it bundles source with dependency context
