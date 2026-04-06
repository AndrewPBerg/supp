---
name: todo
description: "Find TODO, FIXME, HACK, and XXX comments across the codebase with optional git blame and context"
user_invocable: true
---

Run the following command and show the user the output:

```
supp todo --no-copy --no-color --json $ARGUMENTS
```

## When to use

- You need to find actionable tech debt, incomplete work, or known issues in the codebase
- The user asks about TODOs, FIXMEs, or outstanding work items
- Before refactoring: check what known issues exist in the area you're about to change
- During onboarding: get a quick inventory of open work items
- Use `-B` to add git blame (who wrote it, when) — slower but useful for triage
- Use `-C N` to include N surrounding context lines around each match
- Use `-t FIXME,HACK` to filter to specific tag types

## When NOT to use — pick a different supp tool instead

- **Need to find where a symbol is defined?** → Use `/sym`
- **Need to understand how code works?** → Use `/why` or `/ctx`
- **Need project structure?** → Use `/tree`
- **Need to review git changes?** → Use `/diff`

### Examples

- `/todo` — find all TODOs/FIXMEs in the project
- `/todo -t FIXME` — only show FIXME comments
- `/todo -B` — include git blame (author, date) for each match
- `/todo -C 2` — show 2 lines of context around each match
- `/todo -t HACK,XXX -B` — show HACK/XXX comments with blame info
