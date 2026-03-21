# Examples

The `examples/` directory contains a mini project-management app implemented in 6 languages. Every language dir has the same domain — users, projects, tasks — so you can compare how supp handles each one.

```
examples/
├── go/           # structs, interfaces, // doc comments
├── java/         # class hierarchy, extends/implements, Javadoc
├── javascript/   # functions, require() imports, JSDoc
├── python/       # dataclass hierarchy, """docstrings""", from/import
├── rust/         # traits, structs, /// doc comments, use imports
└── typescript/   # interfaces, generics, abstract classes, ES imports
```

## Context generation

The default command bundles files into clipboard-ready context for an LLM.

```bash
# Whole python example
supp examples/python/

# Just models across two languages
supp examples/python/models.py examples/rust/models.rs

# Only .ts files from the examples dir
supp -r '\.ts$' examples/

# Codemap mode — signatures only, 58% smaller
supp --map examples/python/
```

## Interactive file picking with fzf

`supp pick` launches [fzf](https://github.com/junegunn/fzf) for interactive multi-select, then prints selected paths. Compose it with any supp command:

```bash
# Pick files interactively, generate context
supp $(supp pick examples/)

# Pick a single file
supp $(supp pick -s examples/)

# Pick, then view as codemap
supp --map $(supp pick examples/)

# Pre-filter to only Java files, then pick
supp -r '\.java$' pick examples/

# Pick files, pipe to why for a deep-dive
supp why $(supp pick -s examples/)
```

fzf shows a file preview pane. The number of preview lines (default: 100) is configurable in [`supp.toml`](config.md):

```toml
[pick]
preview_lines = 50
```

## Tree view

```bash
# Full tree with git status indicators
supp tree examples/

# Limit depth
supp tree examples/ -d 1

# Just the python dir
supp tree examples/python/
```

## Symbol search

`supp sym` (alias `supp s`) searches the symbol index across all languages:

```bash
# Find everything named "User"
supp sym User

# Search for validation-related symbols
supp sym validate
```

Example output — results ranked by PageRank across the codebase:

```
 cl User                        examples/python/models.py:32   class User(BaseModel):
 st User                        examples/rust/models.rs:14     pub struct User {
 cl User                        examples/java/User.java:10     public class User extends BaseEntity ...
 st User                        examples/go/models.go:18       User struct {
 if UserData                    examples/typescript/models.ts:14  interface UserData extends Entity {
 ...
```

## Symbol deep-dive

`supp why` (alias `supp w`) extracts everything about a symbol: definition, docs, hierarchy, call sites, and dependencies.

### Python class with hierarchy

```bash
supp why BaseModel
```

```
  supp why cl BaseModel  examples/python/models.py:8
  ────────────────────────────────────────

  Root of the model hierarchy.

  All domain models inherit from this to get
  consistent serialization and validation.

  Children
    v User     examples/python/models.py:32
    v Project  examples/python/models.py:53
    v Task     examples/python/models.py:70

  class BaseModel:
      """Root of the model hierarchy. ..."""

      id: Optional[str] = None

      def validate(self) -> bool: ...
      def to_dict(self) -> dict[str, Any]: ...
      ...
```

### Cross-language references

```bash
supp why is_admin
```

The same symbol name appears in Python, Rust, Go, and Java. `why` picks the top-ranked match and shows call sites across every language:

```
  Referenced in 3 locations
    examples/python/service.py:59 in admin_users
    examples/rust/models.rs:54 in is_admin
    examples/rust/service.rs:79 in admin_users
```

### TypeScript generic class

```bash
supp why Store
```

Shows JSDoc, the `extends Entity` parent, the full generic class body, and all references from `service.ts`.

### Java inheritance chain

```bash
supp why User.java
# or just:
supp why User
```

Shows the `extends BaseEntity implements Validatable` hierarchy, Javadoc, and cross-file usage in `ProjectService.java`.

## Combining commands

```bash
# Pick a file, then deep-dive its main class
supp why $(supp pick -s examples/java/)

# Generate slim context for just the Python layer
supp --slim examples/python/

# Diff your changes, filtered to examples only
supp diff -f "examples/*"

# Full context of everything, no clipboard
supp -n examples/
```

## Workflow: feeding an LLM

A typical workflow for giving an LLM context about unfamiliar code:

```bash
# 1. Get the lay of the land
supp tree examples/ -d 2

# 2. Pick the files that matter
supp $(supp pick examples/)

# 3. Deep-dive the symbol you're asking about
supp why BaseModel

# 4. All three are now in your clipboard history —
#    paste them into your LLM conversation
```
