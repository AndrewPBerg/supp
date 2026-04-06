---
name: sym
description: "Find symbol definitions by name with ranked results — use INSTEAD OF Grep/Glob when locating functions, types, structs, traits, or constants.
TRIGGER when: you need to find where a function, type, struct, trait, class, constant, or variable is defined, or locate a symbol by name in the codebase.
DO NOT TRIGGER when: searching for arbitrary text patterns, log messages, config values, or string literals (use Grep for those)"
user_invocable: true
---

Run the following command and show the user the output:

```
supp sym --no-copy --no-color --json $ARGUMENTS
```

## When to use

- You need to find where a function, class, struct, trait, type, or constant is defined
- The user mentions a symbol name and you need to locate it in the codebase
- You want a ranked list of matching symbols with file paths, line numbers, and importance scores
- During planning or before editing: locate symbols first so you have exact paths and line numbers
- Before `/why`: first find the symbol with `/sym`, then deep-dive it with `/why`

## When NOT to use — pick a different supp tool instead

- **Need to understand what a symbol does, not just find it?** → Use `/why` — it shows definition, callers, and dependencies
- **Need to read full file contents?** → Use `/ctx` — it bundles source with dependency context
- **Need project structure?** → Use `/tree`
- **Need to review git changes?** → Use `/diff`

### Examples

- `/sym build_index` — find where build_index is defined
- `/sym --kind function parse` — find all functions matching "parse"
- `/sym Config` — find Config types/structs across the project
