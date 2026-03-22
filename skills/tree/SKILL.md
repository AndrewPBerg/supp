---
name: tree
description: "Show project layout with git status — use when you need to understand directory structure or find where things live"
user_invocable: true
---

Run the following command and show the user the output:

```
supp tree --json $ARGUMENTS
```

## When to use

- You need to orient yourself in an unfamiliar project
- The user asks about project structure or "what's in this directory"
- You want to see which files are modified, added, or untracked at a glance
- Use `-d <N>` to limit depth for large projects
