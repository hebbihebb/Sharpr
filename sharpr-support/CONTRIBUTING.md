# Contributing

## Scope

Until the current stabilization pass lands, prefer support work that stays out of the main GTK implementation files unless there is a clear blocker:

- repository hygiene
- documentation
- verification workflow
- packaging notes
- isolated tests for pure logic

If a change needs to touch active stabilization files, coordinate first and keep the patch narrowly scoped.

## Development Setup

Use [DEVELOPMENT.md](./DEVELOPMENT.md) for Rust and system package prerequisites.

## Branching And Isolation

- Prefer a separate git worktree for parallel efforts.
- Keep support-only work on its own branch.
- Avoid mixing docs, hygiene, and feature code in one patch set.

## Verification Expectations

Before opening a PR, run what is available locally:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build
```

When tests are added:

```bash
cargo test
```

If the environment is missing GTK or Rust toolchain dependencies, note that explicitly in the PR or handoff.

## Low-Risk Contribution Areas

- README and contributor docs
- `.gitignore` and local tooling hygiene
- packaging notes and release preparation docs
- unit tests for pure formatting and path helpers
- follow-up docs for architectural findings

## Change Guidelines

- Keep patches small and reviewable.
- Do not rewrite architecture during support work.
- Prefer concrete notes over aspirational plans.
- Call out assumptions when verification could not be completed locally.
