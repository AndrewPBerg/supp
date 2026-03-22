---
name: sym
description: "Find functions, types, and constants by name — PREFER this over Grep/Glob when locating symbols. Use proactively during planning, exploration, and before any code changes to find definitions fast."
user_invocable: true
---

Run the following command and show the user the output:

```
supp sym --json $ARGUMENTS
```

## When to use — USE THIS PROACTIVELY

**This is faster and more accurate than Grep or Glob for finding symbol definitions.** Prefer `/sym` over searching with Grep whenever you need to locate a function, class, struct, trait, type, or constant.

- **During plan mode**: Use `/sym` to locate every symbol mentioned in the plan before proposing changes. This gives you file paths, line numbers, and PageRank importance scores — far better than guessing.
- **Before reading code**: Instead of grepping for a function name, use `/sym` to jump straight to its definition with ranked results.
- **When the user mentions a symbol**: Immediately run `/sym` to find it — don't waste time with Glob/Grep.
- **When exploring a codebase**: Use `/sym` to map out key types, entry points, and important functions. The PageRank scores tell you which symbols are most central.
- **Before `/why`**: First find the symbol with `/sym`, then deep-dive it with `/why`.

### Examples

- `/sym build_index` — find where build_index is defined
- `/sym --kind function parse` — find all functions matching "parse"
- `/sym Config` — find Config types/structs across the project
