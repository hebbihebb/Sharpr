# Sharpr Agent Instructions

Project-specific instructions for Sharpr (shared workflow, Single-Pick Validation, git hygiene, and shell rules live in global `~/.claude/CLAUDE.md`). Tool stubs (`CLAUDE.md`) defer here.

## Read First

1. `CURRENT_TASKS.md`: active milestone, branch, and next task.
2. `ROADMAP.md`: phase order, Work Order, scope, and acceptance criteria.
3. `docs/reference/product.md`: approved product vision, workflows, UX principles, non-goals, and decision log. Read before any user-facing change.
4. `docs/reference/architecture.md`: modules, threading, persistence, and invariants. Read before any non-trivial change.
5. `docs/reference/development.md`: build, run, test, verification, packaging, and native launcher.
6. `docs/reference/design.md` and `docs/reference/ui-gtk.md`: read before UI work.

`README.md` is for GitHub readers; `docs/archive/` is historical only. Precedence: this file → `product.md` → `ROADMAP.md`. Product decisions change only with user approval and a decision-log entry.

## Product & Scope

Sharpr is a fast, offline Linux app for finding, curating, and preparing wallpaper-grade images (Rust, GTK4, SQLite; moving gradually off Libadwaita and running dark-only per D46). Preserve the viewer, filmstrip navigation, folders, and fingerprint-keyed buckets.

- **Established:** viewer, filmstrip, folders, multi-root libraries, fingerprint buckets, visual search (`src/visual_search/`), Show Similar, typed filters, Compare, duplicates, Crop Copy & batch export (PNG default, explicit WebP/Original, D47), JPEG XL/AVIF/HEIC decoding, explicit trash with Undo.
- **Retired:** manual tags (D1; UI removed in Phase 5; `tags.sqlite3` rows stay intact and follow renames/moves; Main menu → Help → Library Maintenance → "Turn Tags into Buckets…"), quality views (replaced by resolution filters), Resize Copy, Survey (D43). Add no tag UI.
- **Removed, do not reintroduce:** Tasks/queues/pipelines/generation history; AI upscaling/enhancement/tagging; ComfyUI/remote backends; model downloaders; rotate/flip pixel editing; metadata write-back; full-editor features; import workflow; saved searches; batch rename; splash screen; scripting.

## Invariants

- **Originals are never modified.** Outputs are new, uniquely named files created exclusively, never overwriting an existing file. Delete moves to system trash only on explicit user action.
- **User data survives every change.** `~/.local/share/sharpr/` and `~/.config/sharpr/` are irreplaceable. Schema changes are additive; never drop or rewrite legacy tables/keys. Retiring UI never deletes data (sole exception: confirm-first auto-tag cleanup, D17). Back up and verify counts before any migration. Tests use temporary dirs, never real data.
- **Filmstrip stays responsive.** Never block the GTK main thread with decoding, scanning, header reads, hashing, SQLite bulk work, model calls, or exports. Keep visible-thumbnail priority and generation counters. Wrap/extend the thumbnail scheduler; never rewrite it.
- **Disabled folders are excluded** from every scan, index, search, view, and write path.
- **Folders are the source of truth.** Caches, thumbnails, dimensions, hashes, and vectors are rebuildable; tags and bucket membership are user data.
- **Offline only.** No runtime network calls, downloads, or external inference. Sole exception: loopback `llama-server` child process managed by Sharpr for visual search (D23/D24, user-installed models).
- **No generated tags.**

## Project Rules & Git

- Prefer extending proven code to rewriting it; extract focused controllers only when already touching that area. Check GTK, GNOME, and existing crates before adding infrastructure, and justify every new dependency (packaging, Flatpak, maintenance cost).
- Work spanning sessions keeps its plan and progress in `CURRENT_TASKS.md` (keep under one page). A correction given twice belongs here.
- **State vocabulary:** *planned*, *implemented*, *verified*, *accepted*, *committed*, *merged*, *deployed*.
- **Branches:** `feature/<name>`, `fix/<name>`, or `codex/<name>` from `main`; `CURRENT_TASKS.md` names the active one. Never delete archive branches (`preserve/october-12-wip`, `visual-search-trial-wip`, `look-prototype`) without approval. On acceptance, fast-forward/rebase into `main` and delete the branch. Unpushed fix commits may be amended.

## Verification & Deploy

Run GTK tests/screenshots off-screen via `tools/gtk-offscreen.py` (never on the desktop or touching desktop clipboard). Commands live in `docs/reference/development.md`.

| Change | Minimum check |
| --- | --- |
| Docs only | `git diff --check`; referenced paths/commands exist |
| Ordinary code | `cargo build` and targeted `cargo nextest run` |
| Persistence, migrations, export, file ops, trash | Data-path regression + original-byte check, then `./check.sh` |
| Filmstrip, thumbnails, threading | Above + rapid scroll & folder-switch check with before/after bench logs |
| Dependencies | `cargo deny check`, `cargo machete`, regenerate `packaging/cargo-sources.json` |
| Release | `./check.sh`, GTK test run, full Flatpak build (`flatpak-build` skill) |

- Always run `cargo build` in `sharpr/` before handoff.
- **Deploy (standing approval):** the launcher runs `~/.local/opt/sharpr/current`, not the checkout build. Run `tools/install-native-launcher.py` on a committed candidate without asking if `pgrep -x sharpr` finds nothing; if Sharpr is running, ask first.
- Follow global Single-Pick Validation on handoff: evidence, one decisive check if user eyes are needed, Accept / Improve / Reject.

## Machine & Docs Notes

- No `sqlite3` CLI or numpy on this machine; use Python's `sqlite3` module.
- Run `cargo` and `./check.sh` from `sharpr/`. After `cd`, prefer absolute paths.
- `git branch -d` refuses behind-remote branches; confirm `git merge-base --is-ancestor <branch> main`, then `git branch -D`. Remove worktrees from outside them.
- Update `CURRENT_TASKS.md` at checkpoints, `ROADMAP.md` on scope/order changes (archive finished phases to `docs/archive/history.md`), and `docs/reference/` when behavior/invariants change. Keep `CLAUDE.md` to a few lines pointing here.
