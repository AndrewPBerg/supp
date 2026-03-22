---
name: why
description: "Explain a symbol: full definition, docs, call sites, and dependencies — use when you need to deeply understand how something works"
user_invocable: true
---

Run the following command and show the user the output:

```
supp why --json $ARGUMENTS
```

## When to use

- The user asks "what does X do?", "how does X work?", or "explain X"
- You need to understand a function's implementation, who calls it, and what it depends on
- You want to trace the impact of changing a symbol (check its call sites)
- You need to see class/struct hierarchy (parents and children)
- You want doc comments and the full source definition in one view
