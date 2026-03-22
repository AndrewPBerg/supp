---
name: diff
description: "Review git changes with file tree and full patches — use when you need to see what changed on a branch or in the working tree"
user_invocable: true
---

Run the following command and show the user the output:

```
supp diff --json $ARGUMENTS
```

## When to use

- The user asks to review changes, see what's different, or check their work
- You need to understand what was modified before writing a commit message or PR description
- You want a structured view of changes with a file tree, line counts, and unified diffs
- Use `-s` for staged, `-t` for tracked unstaged, `-u` for untracked, `-l` for local vs remote
