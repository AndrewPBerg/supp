---
name: ctx
description: "Bundle source files into structured context — use when you need to read or understand file contents, feed code to analysis, or gather context for a task"
user_invocable: true
---

Run the following command and show the user the output:

```
supp --json $ARGUMENTS
```

## When to use

- You need to read one or more source files with a project tree header for orientation
- The user asks you to look at, review, or understand specific files or directories
- You want full file contents with token estimates so you know how much context you're consuming
- Combine with `--map` (or `-m`) to get just signatures and type definitions — useful for scanning a large directory without reading every line
- Combine with `--slim` to strip comments and collapse blanks for a more compact view
