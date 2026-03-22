---
name: why
description: "Explain a symbol: full definition, docs, call sites, dependencies, and hierarchy — USE THIS during planning to understand impact before making changes. Replaces multiple Read/Grep calls with one command."
user_invocable: true
---

Run the following command and show the user the output:

```
supp why -json $ARGUMENTS
```

## When to use — USE THIS PROACTIVELY

**This replaces 5+ manual Read/Grep calls with a single command.** It gives you the definition, doc comments, all call sites, dependencies, and class hierarchy in one shot. Use it whenever you need to understand a symbol before changing it.

- **During plan mode**: Before proposing changes to a function or type, run `/why` to understand its full impact — who calls it, what it depends on, and what inherits from it. This prevents plans that miss side effects.
- **Before refactoring**: `/why` shows every call site, so you know exactly what will break. Don't manually grep for callers when `/why` already does this.
- **When the user asks "what does X do?"**: `/why` gives you the complete picture — definition, docs, usage, and dependencies — in one call.
- **When assessing change impact**: The call sites and dependency list tell you the blast radius of any modification.
- **For understanding unfamiliar code**: Instead of reading file after file, `/why` gives you the focused context around any symbol.

### Examples

- `/why build_index` — understand build_index: its definition, who calls it, what it depends on
- `/why --symbol Config` — deep-dive the Config type including hierarchy
- `/why parse_arguments` — see full definition, callers, and dependencies before refactoring
