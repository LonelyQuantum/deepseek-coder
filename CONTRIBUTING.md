# Contributing to deepseek-coder

Thanks for helping build `deepseek-coder`. This project is an AGPL-3.0-or-later free software code agent written in Rust and TypeScript.

## Development Setup

Use the setup instructions in `README.md` first. On Windows, verify the toolchain from a fresh PowerShell session:

```powershell
git --version
rustc --version
cargo --version
node --version
pnpm --version
rg --version
```

Install project dependencies from the repository root:

```powershell
pnpm install
```

## Before You Submit

Run the same checks used by CI:

```powershell
pnpm run check
```

For targeted work, these commands are useful:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm -r typecheck
```

## Pull Request Guidelines

- Keep changes focused on one topic.
- Explain the user-visible behavior and the engineering tradeoffs.
- Include tests when changing behavior.
- Update `README.md` development plan checkboxes when completing a planned item.
- Do not commit local secrets, `.env`, `node_modules`, `target`, build outputs, or run logs.
- Commit lockfiles when dependencies change: `Cargo.lock` and `pnpm-lock.yaml`.

## Code Style

- Rust uses `rustfmt`, Clippy, and `unsafe_code = "forbid"`.
- TypeScript uses strict compiler settings and shared workspace configuration.
- Prefer explicit errors over silent fallback behavior.
- Keep protocol types shared instead of duplicating ad hoc shapes across packages.

## Licensing

By contributing, you agree that your contribution is licensed under `AGPL-3.0-or-later`.
