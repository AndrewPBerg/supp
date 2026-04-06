---
name: diff
description: "Review git changes: staged, unstaged, or branch diffs with full patches — use INSTEAD OF git diff in Bash when reviewing work or preparing commits.
TRIGGER when: you need to see what changed, review diffs, check staged/unstaged changes, or prepare a commit message.
DO NOT TRIGGER when: you need project structure (use /tree) or need to understand code logic (use /why or /ctx)"
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

## When NOT to use — pick a different supp tool instead

- **Need project structure, not changes?** → Use `/tree`
- **Need to find where a symbol is defined?** → Use `/sym`
- **Need to understand how code works (not just what changed)?** → Use `/why` or `/ctx`
