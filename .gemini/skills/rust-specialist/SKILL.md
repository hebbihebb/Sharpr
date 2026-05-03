---
name: rust-specialist
description: Expert Rust development for the Sharpr project. Use this skill for any task involving .rs files, cargo configuration, or GTK4/Libadwaita UI implementation in Rust. It ensures zero-regression quality through strict validation workflows and adherence to gtk-rs architectural patterns.
---

# Rust Specialist

## Overview
This skill enables high-precision Rust development tailored for the Sharpr project. It focuses on avoiding common pitfalls such as UI-threading violations, trailing whitespace errors, and test regressions.

## Workflow: Zero-Regression Quality
Whenever you modify Rust code, you MUST follow the [Quality Workflow](references/quality-workflow.md). This includes:
1. **Incremental Checks:** Run `cargo check` and `clippy` before full builds.
2. **Strict Formatting:** Always run `cargo fmt` to eliminate whitespace issues.
3. **Behavioral Verification:** Run `cargo test` and ensure all 120+ tests pass.

## Core Guidelines
- **GTK Architecture:** Strictly follow [GTK Best Practices](references/gtk-best-practices.md) for threading and subclassing.
- **Async/Sync:** Use `async_channel` for thread communication. Never block the GTK main loop.
- **State:** Use `Rc<RefCell<AppState>>` for main-thread shared state.

## Resources
- [Quality Workflow](references/quality-workflow.md): The step-by-step validation process.
- [GTK Best Practices](references/gtk-best-practices.md): Threading and subclassing patterns.
