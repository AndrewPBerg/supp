---
name: ctx
description: "Analyze a file's full context: compressed source, dependencies, and who uses it — use before editing a file to understand its role and connections."
user_invocable: true
---

Run the following command and show the user the output:

```
supp --json $ARGUMENTS
```

## When to use — USE THIS PROACTIVELY

**Use `/ctx` before editing any file to understand its role, dependencies, and who depends on it.** This gives you compressed source + dependency graph in one call.

- **Before modifying a file**: Understand what the file imports, exports, and who references it.
- **During plan mode**: Map out file-level dependencies to plan changes that won't break consumers.
- **When the user points to a file**: Get the full picture before proposing changes.
- Combine with `--map` (or `-m`) to get just signatures and type definitions — useful for scanning a large directory without reading every line.
- Combine with `--slim` to strip comments and collapse blanks for a more compact view.
