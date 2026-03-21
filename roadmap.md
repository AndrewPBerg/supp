# Roadmap

## Done

- [x] Context generation — bundle files/dirs into clipboard-ready LLM context
- [x] Compression modes — `--slim` (strip comments) and `--map` (signatures only, ~58% reduction)
- [x] Diff — git diff with branch comparison, staged/tracked/untracked/local/all modes, regex filtering
- [x] Tree — directory tree with git status indicators
- [x] Shell completions (bash, zsh, fish)
- [x] fzf integration (`supp pick`) — interactive multi-select file picker, composable with all commands
- [x] Config system — hardcoded defaults, CLI flags always win
- [x] Symbol search (`supp sym`) — PageRank-ranked index across Rust, Python, TS, TSX, JS, Go, C, C++, Java, JSON, Markdown
- [x] Symbol deep-dive (`supp why`) — full definition, doc comments, call sites, deps, class hierarchy
- [x] TSX/JSX component-aware `why` — arrow function components, props interface linking, hook deps, JSX element tracking
- [x] C/C++ support for `why` — `#include` resolution, header symbol scanning, class hierarchy (`: public Base`), `Foo::bar()` scope-qualified methods
- [x] Multi-language examples directory (8 languages) for testing and demos
- [x] Token estimation — cl100k_base token count on every output
- [x] `supp ctx` — fzf-powered single-file context (pick → context in one step)
- [x] Regex file filtering (`-r`) — works globally across context, diff, pick, tree
- [x] Claude Code skills — `/project:diff`, `/project:ctx`, `/project:why`, `/project:sym`, `/project:tree`
- [x] MCP server (`supp mcp`) — 6 tools over stdio for autonomous AI context gathering
- [x] Incremental `why` — reuse cached symbol index, skip unchanged files
- [x] `--json` flag — machine-readable JSON output across all commands for editor/IDE integrations
- [x] Smarter hierarchy — multi-level inheritance chains, mixin/trait resolution
- [x] Pipe-friendly output — detect `| pipe` and skip color/clipboard automatically
