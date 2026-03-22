---
name: sym
description: "Find functions, types, and constants by name — use when you need to locate a symbol before reading or editing it"
user_invocable: true
---

Run the following command and show the user the output:

```
supp sym --json $ARGUMENTS
```

## When to use

- You need to find where a function, class, struct, trait, or constant is defined
- The user mentions a symbol name and you need to locate it in the codebase
- You want a ranked list of matching symbols across all languages in the project
- Use this before `/why` — first find the symbol, then deep-dive it
