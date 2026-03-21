# Roadmap

## Done

- [x] Context generation — bundle files/dirs into clipboard-ready LLM context
- [x] Diff — git diff with branch comparison, staged/unstaged/untracked modes
- [x] Tree — directory tree with git status indicators
- [x] Shell completions (bash, zsh, fish)
- [x] fzf integration (`supp pick`) — interactive multi-select file picker
- [x] Config system — global + local `supp.toml` with field-level merge
- [x] Compression modes — `--slim` (strip comments) and `--map` (signatures only)
- [x] Symbol search (`supp sym`) — PageRank-ranked symbol index across 9+ languages
- [x] Symbol deep-dive (`supp why`) — full definition, doc comments, call sites, deps, hierarchy
- [x] Multi-language examples directory for testing and demos

## Next

- [ ] `supp ctx` — fzf-powered single-file context (pick → context in one step)
- [ ] TSX/JSX component-aware `why` — props interfaces, hook dependencies
- [ ] C/C++ support for `why` — `#include` tracking, header/source pairing
- [ ] Incremental `why` — reuse cached symbol index, skip unchanged files
- [ ] `supp why --json` — machine-readable output for editor integrations
- [ ] `supp sym --kind fn` — filter symbol search by kind (fn, class, trait, etc.)
- [ ] Smarter hierarchy — multi-level inheritance chains, mixin/trait resolution
- [ ] `supp diff --why <symbol>` — diff scoped to a symbol and its callers
- [ ] Remote repo support — `supp why <symbol> --repo <url>` for quick lookups
- [ ] Language server protocol (LSP) integration — use LSP for go-to-definition when available
- [ ] fzf symbol picker — `supp pick --symbols` to browse and select from the symbol index
- [ ] `supp pick` preview modes — show codemap or symbol list in the fzf preview pane
- [ ] Pipe-friendly output — detect `| pipe` and skip color/clipboard automatically
