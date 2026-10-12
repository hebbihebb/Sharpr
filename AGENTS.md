# Sharpr Agent Instructions

The authoritative instructions for every coding agent working on this repository. Tool-specific files (`CLAUDE.md`, skills, memories) defer to this file.

## Read First

1. `CURRENT_TASKS.md`: Git and working-tree state, the active milestone, and the next task.
2. `ROADMAP.md`: the only roadmap, including phase order, scope, and acceptance criteria.
3. `docs/reference/product.md`: the approved product vision, workflows, UX principles, non-goals, and decision log. Read it before any user-facing change.
4. `docs/reference/architecture.md`: modules, threading, persistence, and invariants. Read it before any non-trivial change.
5. `docs/reference/development.md`: build, run, test, verification, packaging, and the native launcher.
6. `docs/reference/ui-gtk.md`: read it before UI work.

`README.md` is for GitHub readers. `docs/archive/` is historical evidence, not instructions. If documents disagree, this file wins, then `product.md`, then `ROADMAP.md`. Product decisions change only with the user's approval and a decision-log entry.

## Product

Sharpr is a fast, offline Linux app for finding, curating, and preparing wallpaper-grade images (Rust, GTK4, Libadwaita, SQLite). Its strengths are the viewer, filmstrip navigation, folders, and buckets. Buckets are hand-curated collections, currently backed by tags. Preserve all of these.

Images will be found through visual search and typed filters, not manual tags, and preparation is crop plus batch WebP export. Details are in `docs/reference/product.md`.

## Invariants

- **Originals are never modified.** No code path writes, re-encodes, renames, or moves a source image. Outputs are new, uniquely named files created exclusively, never overwriting an existing file. Delete moves files to the system trash only after an explicit user action.
- **User data survives every change.** Tags, buckets, and settings in `~/.local/share/sharpr/` and `~/.config/sharpr/` are irreplaceable. Schema changes are additive. Never drop or rewrite legacy tables or keys.
  - Retiring a feature from the UI never deletes its data. The one approved exception is the backed-up, confirm-first automatic-tag cleanup (decision D17).
  - Back up and verify counts before any data migration.
  - Tests and experiments use temporary directories, never the real data directory.
- **The filmstrip stays responsive.** Never block the GTK main thread with decoding, scanning, header reads, hashing, SQLite bulk work, model calls, or exports. Keep visible-thumbnail priority and generation counters. Wrap or extend the thumbnail scheduler; do not rewrite it.
- **Disabled folders are excluded** from every scan, index, search, view, and write path.
- **Folders are the source of truth.** Caches, thumbnails, dimensions, hashes, and vectors are rebuildable. Tags and bucket membership are user data.
- **The app is offline.** It makes no runtime network calls, downloads no models, and calls no external inference services. The only exception is the experimental visual-search trial, which may talk over loopback to a child process the app manages.
- **No generated tags.** The app creates no descriptive, format, or resolution tags. Existing assignments stay intact.

## Scope

- **Established:** viewer, filmstrip, folders, buckets, search, filters, Compare, duplicates, Crop/WebP Copy, and explicit trash.
- **Frozen:** manual tags (D1). Do no tagging improvements. Data-safety fixes are allowed. Tags are retired from the UI only after visual search passes `ROADMAP.md` Phase 3.
- **Being replaced:**
  - Quality views give way to resolution filters (Phase 2).
  - Resize Copy and non-WebP output give way to batch WebP export (Phase 6).
- **In progress:** the roadmap phases.
- **Experimental:** visual search. The unfinished trial wiring is on the `visual-search-trial-wip` branch, behind the `visual-search-trial` Cargo feature. It is off by default and excluded from the Flatpak until `ROADMAP.md` Phase 5 promotes it. Keep experimental code in its own module behind the feature, and never let it change default behavior.
- **Removed, do not reintroduce:** Tasks, processing queues, pipelines, and generation history; AI upscaling, enhancement, and tagging; ComfyUI and remote backends; model downloaders; rotate/flip pixel editing; metadata write-back; full-editor features; import workflow; saved searches; batch rename; splash screen; scripting. Enhancement and upscaling stay in external tools. The product non-goals in `product.md` also apply.

## Working Rules

- **Start from the repository, not chat history.** Read `CURRENT_TASKS.md` and run `git status`. If you find uncommitted work you did not create, identify it before you touch anything.
- **Size tasks by outcome:** goal, effort, verification, and essential guardrails. For work spanning sessions, keep the durable plan in `CURRENT_TASKS.md`.
- **Prefer extending proven code to rebuilding it.** Extract a focused controller only when you are already changing that area. No broad rewrites. Before writing infrastructure, check GTK, GNOME, and existing crates.
- **Justify every dependency** by its packaging, Flatpak, and maintenance cost.
- **Finish, then stop.** Do one self-review per milestone. Fix real defects, record the rest in `ROADMAP.md` under Backlog, and close the milestone when its acceptance criteria are met. Do not start review, audit, and remediation loops, and do not write dated audit documents into the repository.
- **Report state precisely** using these words:
  - *planned*: in the roadmap
  - *implemented*: in the working tree
  - *verified*: checks passed
  - *accepted*: the user tested it
  - *committed*: on `viewer-reset`
  - *merged*: on `main`
  - *deployed*: built into the binary the user launches

## Git

- `viewer-reset` is the development line. `main` is the older line from before the July 2026 reset. It is fast-forwarded only after user acceptance (`ROADMAP.md` Phase 0).
- Commit each user-testable unit, and keep experimental work in separate commits. Never end a session with large uncommitted work. Either commit it or record its state in `CURRENT_TASKS.md`.
- A fix to an unpushed commit may be amended into that commit.
- Never push, merge into `main`, deploy, delete branches, or touch user data without explicit user approval.
- The user has no Git background. Make the Git decisions yourself and explain them in plain language.

## Verification

Check in proportion to the risk. `docs/reference/development.md` has the commands.

| Change | Minimum check |
| --- | --- |
| Docs only | `git diff --check`; referenced paths and commands exist |
| Ordinary code | `cargo build` and targeted `cargo nextest run` tests |
| Persistence, migrations, export, file operations, or trash | Regression test of the data path, original-byte preservation where relevant, then `./check.sh` |
| Filmstrip, thumbnails, or threading | Above, plus a manual rapid scroll and folder-switch check on a large folder, with before/after bench logs |
| Dependencies | `cargo deny check` and `cargo machete`; regenerate `packaging/cargo-sources.json` |
| Release | `./check.sh`, a GTK test run, and a full Flatpak build |

Before handing work back, always run `cargo build` in `sharpr/`. Then tell the user exactly what to test and what should happen. The desktop launcher runs `sharpr/target/debug/sharpr` from the main checkout, so a build there changes what the user launches. Say so whenever it happens.

## Documentation

- `CURRENT_TASKS.md`: update it at every checkpoint. Keep it under one page.
- `ROADMAP.md`: update it when scope, order, or acceptance criteria change. Move finished phases to `docs/archive/history.md` as a short summary.
- `docs/reference/`: update it when architecture, commands, or invariants change.
- Keep agent-specific files to a few lines that point here.
