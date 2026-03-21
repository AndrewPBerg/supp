# supp

[![CI](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml/badge.svg)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/AndrewPBerg/supp/branch/main/graph/badge.svg)](https://codecov.io/gh/AndrewPBerg/supp)
[![clippy](https://img.shields.io/badge/clippy-passing-brightgreen?logo=rust)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![fmt](https://img.shields.io/badge/fmt-checked-brightgreen?logo=rust)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![audit](https://img.shields.io/badge/audit-passing-brightgreen?logo=rust)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![deny](https://img.shields.io/badge/deny-passing-brightgreen?logo=rust)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)

A code-aware supplemental context tool for LLMs — extracts symbols, dependencies, token counts, and structural diffs from your codebase.

## Quality Gates

| Tool | Purpose |
|------|---------|
| `cargo fmt` | Enforces consistent formatting |
| `cargo clippy` | Lint checks with `-D warnings` |
| `cargo tarpaulin` | Code coverage uploaded to Codecov |
| `cargo audit` | Checks dependencies for known vulnerabilities |
| `cargo deny` | License compliance, ban, and source checks |
