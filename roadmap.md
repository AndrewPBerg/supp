- [ ] Get Diff working 90%
  - [ ] fix formatting and align of print, it needs to be right aligned and padding for standardization for even very long file names 
  ```bash bad example of bad align
  ➜ cargo run diff -c
      Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.03s
       Running `target/debug/supp diff -c`
  
    supp diff  Staged  HEAD ... index  (diff-functionality)
    ────────────────────────────────────────
  
  ├── CLAUDE.md                  deleted     +0 -324
  ├── Cargo.lock                 modified   +99   -9
  ├── Cargo.toml                 modified    +1   -0
  ├── cool-ideassssssssssssss.md added       +4   -0
  └── future.md                  deleted     +0  -59
  ├── docs/
  │   └── diff.md added      +82   -0
  └── src/
      ├── cli.rs  modified    +4   -0
      ├── diff.rs deleted     +0   -0
      ├── git.rs  modified  +175  -61
      ├── main.rs modified  +189   -6
      └── tree.rs deleted     +0   -0
  
    11 files  (2 added, 5 modified, 4 deleted)   +554 -459
  
    ✓ Copied to clipboard (42.7 KB)
  ```
- [ ] Get Tree working 90%, simple easy output that is auto copied and clean w/ configurable levels, ignores what git .ignores ofc
- [ ] Get supp context working 90% w/ l
- [ ] Get tiktoken estimates going, run in parallel with all current diff tree and root supp args to display estimated token context if passed
- [ ] Configurable ~/.supp/config.toml with defaults that override, needs fleshing out ofc
