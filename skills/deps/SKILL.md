---
name: deps
description: "Visualize file-level import/dependency graph — use when you need to understand what a module depends on or what depends on it.
TRIGGER when: you need to understand file-level dependencies, what imports what, or assess blast radius of changing a module.
DO NOT TRIGGER when: you need symbol-level dependencies (use /why) or just need to read file contents (use /ctx)"
user_invocable: true
---

Run the following command and show the user the output:

```
supp deps --json $ARGUMENTS
```

## When to use

- The user asks "what depends on this file?" or "what does this module import?"
- You need to understand the dependency chain before refactoring or moving files
- You want to assess the blast radius of changing a module (use `-R` for reverse deps)
- Onboarding: understand how files relate to each other in an unfamiliar project
- Use `-d <N>` to limit traversal depth for large projects
- Use `--dot` to generate Graphviz DOT output for visualization

## When NOT to use — pick a different supp tool instead

- **Need symbol-level dependencies (what a function calls)?** -> Use `/why` — it shows per-symbol deps, callers, and hierarchy
- **Need to find where a symbol is defined?** -> Use `/sym`
- **Need to read file contents?** -> Use `/ctx`
- **Need project structure without dependency info?** -> Use `/tree`
- **Need to review git changes?** -> Use `/diff`

### Examples

- `/deps src/main.rs` — what files does main.rs depend on (tree view)
- `/deps src/main.rs -R` — what files depend on main.rs (reverse deps)
- `/deps src/main.rs -d 1` — direct dependencies only, no transitive
- `/deps --dot` — whole-project graph in DOT format for Graphviz
- `/deps` — whole-project file dependency graph
