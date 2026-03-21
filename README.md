# supp

[![CI](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml/badge.svg)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/AndrewPBerg/supp/branch/main/graph/badge.svg)](https://codecov.io/gh/AndrewPBerg/supp)
[![clippy](https://img.shields.io/badge/clippy-passing-brightgreen?logo=rust)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![fmt](https://img.shields.io/badge/fmt-checked-brightgreen?logo=rust)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![audit](https://img.shields.io/badge/audit-passing-brightgreen?logo=rust)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)
[![deny](https://img.shields.io/badge/deny-passing-brightgreen?logo=rust)](https://github.com/AndrewPBerg/supp/actions/workflows/ci.yml)

A code-aware supplemental context tool for LLMs — extracts symbols, dependencies, token counts, and structural diffs from your codebase.

## Install

### Quick install (latest release)

```sh
curl -fsSL https://raw.githubusercontent.com/AndrewPBerg/supp/main/install.sh | bash
```

### Specific version

```sh
VERSION=0.1.0 curl -fsSL https://raw.githubusercontent.com/AndrewPBerg/supp/main/install.sh | bash
```

### Custom install directory

```sh
INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/AndrewPBerg/supp/main/install.sh | bash
```

### From crates.io

```sh
cargo install supp
```

### From source

```sh
git clone https://github.com/AndrewPBerg/supp.git
cd supp
cargo install --path .
```

### Supported platforms

| Target | OS | Arch |
|--------|----|------|
| `x86_64-unknown-linux-gnu` | Linux | x86_64 |
| `aarch64-unknown-linux-gnu` | Linux | ARM64 |
| `x86_64-unknown-linux-musl` | Linux (Alpine/musl) | x86_64 |
| `x86_64-apple-darwin` | macOS | Intel |
| `aarch64-apple-darwin` | macOS | Apple Silicon |
| `x86_64-pc-windows-msvc` | Windows | x86_64 |

Windows users: download the `.zip` from [GitHub Releases](https://github.com/AndrewPBerg/supp/releases) and add `supp.exe` to your PATH.

## Managing supp

```sh
# Check installed version (auto-checks for updates)
supp version

# Update to the latest release
supp update

# Remove supp from your system
supp uninstall
```

## Quality Gates

| Tool | Purpose|
|------|---------|
| `cargo fmt` | Enforces consistent formatting |
| `cargo clippy` | Lint checks with `-D warnings` |
| `cargo tarpaulin` | Code coverage uploaded to Codecov |
| `cargo audit` | Checks dependencies for known vulnerabilities |
| `cargo deny` | License compliance, ban, and source checks |
