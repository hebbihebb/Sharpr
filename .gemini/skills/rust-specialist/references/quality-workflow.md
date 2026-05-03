# Rust Quality Workflow (Zero-Regression)

Follow this workflow for every change to `.rs` files to ensure structural integrity and code quality.

## 1. Surgical Modification
- Apply changes to the relevant files.
- Ensure no trailing whitespace is introduced.
- Maintain existing coding style (naming, typing, comments).

## 2. Local Validation (Incremental)
Run these commands from the `sharpr/` directory:

```bash
# Compile and check for immediate syntax/type errors
GSETTINGS_SCHEMA_DIR=data cargo check

# Run lints (must pass without warnings)
GSETTINGS_SCHEMA_DIR=data cargo clippy -- -D warnings
```

## 3. Formatting & Cleanup
```bash
# Automatically format and remove trailing whitespace
cargo fmt

# Manual whitespace check if fmt is not enough
# find . -name "*.rs" | xargs sed -i 's/[[:space:]]*$//'
```

## 4. Verification (Behavioral)
```bash
# Build the project
GSETTINGS_SCHEMA_DIR=data cargo build

# Run ALL tests to ensure no regressions
GSETTINGS_SCHEMA_DIR=data cargo test
```

## 5. Failure Recovery
If a test fails or clippy complains:
1. Fix the issue immediately.
2. If the change was part of a larger task, amend the fix to the same commit (if not yet pushed).
3. Re-run the entire quality workflow from Step 2.
