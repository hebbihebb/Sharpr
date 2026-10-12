# Current Tasks

## Active Milestone

`ROADMAP.md` Phase 0, Checkpoint and trustworthy baseline. Nothing else starts until it is done.

## State Snapshot (October 12, 2026)

**Branches:**
- **`viewer-reset`** is the development line. It is at `642665d` and was pushed to `origin/viewer-reset`.
- **`main`** is at `2679e44`, from before the July reset. It is 9 commits behind `viewer-reset` and can be fast-forwarded.
- **`docs-realignment`** held this documentation realignment and was fast-forwarded into `viewer-reset`. With the user's approval, both the branch and its worktree `../Sharpr-docs` can be deleted.
- **`viewer-base`** (`f1d937c`, worktree `../Sharpr-Base`) marks the July reset baseline.
- **Six older local branches** are fully merged into `main`: `advanced-queue`, `fix/folder-switch-performance`, `hig-writing-style-audit`, `improve-tag-popover`, `improve-upscale-save-flow`, and `quality-assessment`.

**Implemented but not committed** in the main checkout: about 2,400 changed lines in 23 files, plus untracked files.

- **New files:**
  - `model/browse.rs`
  - `ui/crop_dialog.rs`
  - `ui/window/{search,tag}_controller.rs`
  - `ui/window/gtk_tests.rs`
  - `tools/benchmark-*.py`
  - `docs/research/image-search.md`
- **What it does:**
  - scoped search
  - aspect and resolution filters
  - manual-tag persistence fixes
  - Crop and WebP copies with export safety fixes
  - an unfinished experimental visual-search trial:
    - `Cargo.toml` adds the `visual-search-trial` feature with `ureq` and `sha2`
    - `main.rs` declares a `semantic_trial` module whose file does not exist yet
- **Verification:**
  - On October 12, before the trial wiring, build, clippy, 134 tests, the GTK test, deny, and machete passed.
  - `./check.sh` will now fail until the trial module exists, because it builds with `--all-features`.
- **Superseded doc edit:** the uncommitted one-line "evaluation exception" in `AGENTS.md` was discarded when the documentation was merged. The Scope section of the new `AGENTS.md` covers it.

**Deployed:**
- The desktop launcher `sharpr-dev.desktop` runs `sharpr/target/debug/sharpr` from the main checkout. That binary was last built at 20:09 on October 12 from the uncommitted tree.
- No Flatpak is installed.

**User data:**
- `~/.local/share/sharpr/` holds 4,539 indexed images, 6 buckets, and 7,925 tag rows; 7,565 of those rows are legacy automatic tags.
- It was last written on September 27, so October work has not touched it.
- About 5,800 images live under `~/Pictures`, on a non-removable disk.

**Decisions recorded with the user on October 12:**
- The visual-search trial stays, as an off-by-default experiment after the ordinary work is committed.
- `main` is fast-forwarded after acceptance.
- Gemini and Antigravity are no longer used.
- Legacy automatic tags get a one-time cleanup, backed up and confirmed in the app.
- No V2 rewrite (see `ROADMAP.md`, Architecture Decision).

## Next Task: Phase 0, Steps 2–3

Step 1, folding in the documentation, is done.

**Goal:** Turn the October 12 working tree into committed, green, reviewable units on `viewer-reset`, with no behavior changes beyond what is already implemented.

**Effort:** One session. Committing and making the build green, not new features.

**Steps:**

1. Resolve the trial wiring so that both `./check.sh` and `cargo build` (default features) pass. Either finish the minimal `semantic_trial` module behind its feature, or stash the trial wiring onto a `visual-search-trial` branch.
2. Commit the ordinary work as units (a)–(c) from Phase 0 step 3, and the trial as (d), separately or on its own branch.

**Acceptance:**
- `git status` is clean on `viewer-reset`, apart from ignored paths.
- `cargo build`, `./check.sh`, and the GTK test pass at the head (see `docs/reference/development.md`).
- `cargo build --features visual-search-trial` passes, if the trial was committed.
- Each commit builds on its own.
- No file under `~/.local/share/sharpr/` was touched.

**Guardrails:**
- Do not start a new audit round. Unresolved findings already map to Phases 1, 2, and 5.
- Do not push or merge into `main`.
- Tell the user when the launcher's binary is rebuilt.

**After this:** Phase 0 steps 4–7 cover the backup, release launcher, performance baseline, user acceptance, and fast-forwarding `main`. Then comes Phase 1, data-safety repairs.
