# supp why

Deep-dive a symbol: full definition, doc comments, call sites, dependencies, and class hierarchy. Alias: `supp w`.

## Usage

```
supp [-n] why <symbol...>
```

## Global Flags

| Flag | Short | Description |
|------|-------|-------------|
| `--no-copy` | `-n` | Show results only, skip clipboard copy |
| `--no-color` | | Disable colored output |
| `--json` | | Output as JSON (machine-readable) |

## What it extracts

| Section | Description |
|---------|-------------|
| **Doc comment** | Language-aware: Python `"""docstrings"""`, Rust `///`, Go/C/C++ `//`, Java/JS/TS `/** */` |
| **Full definition** | Complete source text of the symbol (not just the signature) |
| **Hierarchy** | For classes/structs: parent classes and child classes found in the project |
| **Call sites** | Every file + line where the symbol is referenced, with caller context |
| **Dependencies** | Symbols used inside the definition, resolved against the project index and imports |

## How it works

1. Finds the symbol using the existing index (exact match first, then fuzzy)
2. Re-parses the source file with tree-sitter to extract the full definition node
3. Extracts doc comments using language-specific rules
4. Scans all project files for references (excluding the definition itself)
5. Collects identifiers from the definition body and resolves them against:
   - The project symbol index
   - File-level imports (Python `from/import`, Rust `use`, JS/TS `import`, C/C++ `#include`)
6. For classes/structs, finds parent and child classes via AST and signature analysis

### TSX/JSX component awareness

- Arrow function components (`const Button = () => { ... }`) are indexed and searchable
- Props interfaces are detected as dependencies (e.g. `ButtonProps` in `({ label }: ButtonProps)`)
- JSX element usage (`<UserCard />`) is tracked as both a call site and a dependency
- React hooks (`useState`, `useEffect`) show as external dependencies from `react`
- Custom hooks (`useAuth`) resolve to their project definition

### C/C++ support

- `#include "file.h"` is resolved relative to the file, project root, and `include/` directories
- Symbols declared in included headers are mapped as dependencies
- `#include <stdlib.h>` system includes are shown as external dependencies
- C++ class hierarchy via `: public Base` is fully supported (parents + children)
- Out-of-class method definitions (`Foo::bar()`) link back to their class

Everything is formatted and copied to the clipboard as structured context for an LLM.

## Supported languages

Rust, Python, TypeScript, TSX, JavaScript, Go, Java, C, C++, JSON, Markdown.

## Examples

```bash
# Look up a function
supp why parse_config

# Look up a class
supp why GitDiff

# Multi-token query
supp why extract doc comment

# Use the alias
supp w Handler

# Print without copying
supp w -n MyClass
```

## Example output

```
  supp why  fn  parse_config  src/config.rs:15
  ────────────────────────────────────────

  Parse the supp configuration from disk,
  merging global and local config files.

  pub fn parse_config(root: &Path) -> Config {
      let global = load_global_config();
      let local = load_local_config(root);
      merge(global, local)
  }

  Referenced in  4 locations
    src/main.rs:23         main
    src/main.rs:45         run
    src/config.rs:102      (test) test_parse_config
    src/cli.rs:88          resolve_depth

  Dependencies  3 symbols
    fn   load_global_config  src/config.rs:30
    fn   load_local_config   src/config.rs:55
    fn   merge               src/config.rs:70

  ✓ Copied to clipboard (1.2 KB)
  ≈ ~384 tokens (est.)
  Done in 45ms
```

### Class with hierarchy

```
  supp why  class  HttpClient  src/client.py:12
  ────────────────────────────────────────

  """HTTP client with retry and auth support."""

  Parents
    ^ BaseClient    src/base.py:5
    ^ AuthMixin     src/mixins.py:20  (requests)

  Children
    v MockClient    tests/conftest.py:8

  class HttpClient(BaseClient, AuthMixin):
      def __init__(self, base_url: str):
          ...
      ...  (42 more lines)

  Referenced in  6 locations
    ...

  Dependencies  5 symbols
    fn   retry_with_backoff   src/utils.py:15
    class BaseClient          src/base.py:5
    --   AuthMixin            (requests)
    ...
```

External dependencies (from imports that don't resolve to project files) are shown with `--` and the module path in parentheses.

### React component with hooks

```
  supp why fn Button  examples/tsx/Button.tsx:5
  ────────────────────────────────────────

  /** A styled button with click tracking. */

  Button = ({ label, onClick, disabled, variant = "primary" }: ButtonProps) => {
    const [clicks, setClicks] = useState(0);
    ...
  }

  Referenced in 3 locations
    examples/tsx/UserCard.tsx:2    import Button from "./Button"
    examples/tsx/UserCard.tsx:11   {onEdit && <Button label="Edit" onClick={onEdit} />}
    examples/tsx/Button.tsx:24    export default Button;

  Dependencies  2 symbols
    if ButtonProps  examples/tsx/types.tsx:2
    -- useState  (react)
```

Props interfaces, JSX element references, and React hooks are all tracked automatically.

### C++ class hierarchy

```
  supp why cl Shape  examples/cpp/include/shape.hpp:7
  ────────────────────────────────────────

  /** Abstract base class for all shapes. */

  Children
    v Circle  examples/cpp/include/circle.hpp:7
    v Rect    examples/cpp/include/rect.hpp:7

  class Shape {
  public:
      virtual ~Shape() = default;
      virtual double area() const = 0;
      virtual double perimeter() const = 0;
      virtual std::string describe() const = 0;
  }

  Referenced in 4 locations
    examples/cpp/include/circle.hpp:7   class Circle : public Shape
    examples/cpp/include/rect.hpp:7     class Rect : public Shape
    examples/cpp/src/main.cpp:7         void print_shape(const Shape& s)
    examples/cpp/src/main.cpp:15        vector<unique_ptr<Shape>> shapes;
```

Out-of-class methods like `Circle::area()` show their parent class and resolve `#include` dependencies from headers.
