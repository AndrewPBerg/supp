---
name: why
description: "Deep-dive a symbol: definition, docs, callers, dependencies, and hierarchy — use INSTEAD OF multiple Read/Grep calls when you need to understand a symbol's role and impact.
TRIGGER when: you need to understand what a function/type/symbol does, who calls it, what it depends on, or its blast radius before refactoring.
DO NOT TRIGGER when: you just need to find where a symbol is defined (use /sym) or read a whole file (use /ctx)"
user_invocable: true
---

Run the following command and show the user the output:

```
supp why --no-copy --no-color --json $ARGUMENTS
```

## When to use

- The user asks "what does X do?", "how does X work?", or "explain X"
- You need to understand a function's implementation, who calls it, and what it depends on
- Before refactoring: see every call site so you know what will break
- You want to assess the blast radius of a change (callers + dependencies in one view)
- You need to see class/struct hierarchy (parents and children)
- For understanding unfamiliar code: definition, docs, usage, and dependencies in one call

## When NOT to use — pick a different supp tool instead

- **Just need to find where a symbol is defined?** → Use `/sym` — faster for locating without full analysis
- **Need to read full file source, not just one symbol?** → Use `/ctx`
- **Need project structure?** → Use `/tree`
- **Need to review git changes?** → Use `/diff`

### Examples

- `/why build_index` — understand build_index: its definition, who calls it, what it depends on
- `/why --symbol Config` — deep-dive the Config type including hierarchy
- `/why parse_arguments` — see full definition, callers, and dependencies before refactoring
