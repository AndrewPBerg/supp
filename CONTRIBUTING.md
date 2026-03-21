# Contributing

## Local Setup

1. **Clone the repo**

   ```sh
   git clone https://github.com/YOUR_USER/supp.git
   cd supp
   ```

2. **Install Rust** (if you haven't already)

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. **Install prek** (pre-commit hook manager)

   ```sh
   cargo install prek
   ```

4. **Install git hooks**

   ```sh
   prek install
   ```

   This sets up pre-commit hooks that run `cargo fmt --check` and `cargo clippy` automatically before each commit.

5. **Build and test**

   ```sh
   cargo build
   cargo test
   ```

6. **Locally use the CLI**
  
  ```sh
  cargo install --path .
```

## Pre-commit Hooks

We use [prek](https://github.com/j178/prek) to run checks before each commit:

- **cargo fmt** — ensures consistent formatting
- **cargo clippy** — catches common mistakes and lint warnings

If a commit is blocked, fix the issues and try again. To auto-fix formatting:

```sh
cargo fmt
```

To run hooks manually on all files:

```sh
prek run --all-files
```
