# Current Tasks

## Active Milestone

**Phase 4: file identity and buckets** (`ROADMAP.md`). Phase 3 finished on October 13 (go with EmbeddingGemma, D22). The Phase 2 follow-ups are in the roadmap; the slow-folder-open logging awaits a session log from a deployed build.

Plan, one commit per testable unit:
1. **Fingerprints.**
   - New table `file_fingerprints(path, size, mtime_ns, hash)`, keyed by path and checked against size and mtime. The hash is xxh3-128 from the new MIT dependency `twox-hash`.
   - A background worker walks the active library, skipping disabled folders, at nice 19 and idle I/O priority. It drops page cache after each file.
   - Progress shows in Preferences.
   - Library index `user_version` 2: older builds refuse it, so rolling back means restoring the backup.
2. **Membership.**
   - New table `bucket_members(collection_id, last_path, fingerprint)`, deleted together with its bucket.
   - A member is any file at `last_path`, or any file with the same fingerprint, so identical copies all count. Members found nowhere count as missing.
   - **Inheritance:** a bucket shows its own members and its descendants'.
   - **Edits:** adding to a child doesn't add to the parent. Removing from a bucket removes from it and its sub-buckets. Rename and reparent change no members. No tag writes.
   - **Migration:** backup first. Each bucket's direct members become every stored effective-tag member, missing files included. Visible counts must match before commit. Tag rows are untouched.
   - Every reader switches over: bucket open, sidebar counts, Info chip, badge colours, Ctrl+B, the menus, create, export, and import.
3. **Re-link and missing.**
   - After fingerprinting, a member whose path is gone moves to a file with its fingerprint.
   - A member whose file changed gets the new fingerprint.
   - Opening a bucket says how many members are missing.

## State (October 13, 2026)

- **Git:**
  - `main` is at `1924346`: the accepted build `66f3d94` plus its notes.
  - `viewer-reset` continues from there with the audit fixes and later work.
  - Nothing is pushed. Pushing is the user's call.
- **Deployed:** `e9c142f` (accepted work plus the audit fixes) at `~/.local/opt/sharpr/e9c142f`, with `current` pointing to it. Not yet tested by the user.
  - **Smoke test:** run on a copy of the live data, with identical tags, buckets, and image list afterwards.
  - **Rollback:** `ln -sfn ~/.local/opt/sharpr/66f3d94 ~/.local/opt/sharpr/current`.
  - Redeploy only with approval, via `python3 tools/install-native-launcher.py`.
- **Backups**, each byte-verified and integrity-checked:
  - `~/.local/share/sharpr.backup-20261013-090919/`, taken before `e9c142f`: 7,718 tag rows, 6 buckets.
  - `~/.local/share/sharpr.backup-20261013-042010/`
  - `~/.local/share/sharpr.backup-20261012-221640/`
- **Live data:** 6 buckets (Model, Art › Diffusion, upgrade?, CSR, Trash?). Of the 7,718 tag rows, 7,346 are old automatic tags; the cleanup is ready but not run.
- **Archive branches, kept:** `preserve/october-12-wip` and `visual-search-trial-wip`.
- **Product direction:** `docs/reference/product.md` (D1–D22). Tags are frozen; the Tags page has been removed (D20). Visual search goes ahead with EmbeddingGemma 2 (D22).

## Next User Check (Build `e9c142f`, or `f619d64` If Deployed)

1. Scroll fast in Wallpapers and switch between big folders; it should feel as before.
2. Open a bucket and start typing a search straight away; the results come from that bucket.
3. Add a visible image to a coloured bucket (Ctrl+B); its number pill takes the colour at once.
4. Remove an image from Art; the corner chip no longer lists Art › Diffusion.
5. With `f619d64`: open a very large image (8K or bigger). It appears without a freeze and looks sharp. Zooming in to 100% sharpens it fully, possibly after a short pause.

## Performance Baseline (Phase 0, Build `5832010`, October 12)

The log is `~/.cache/sharpr/logs/run-1791843839575-1067250.jsonl`.

| Measure | Result |
| --- | --- |
| Startup to first folder ready | 498 ms |
| Folder open, 2,216 images | 116–162 ms; one outlier at 2,449 ms (recurred on October 13, cause open) |
| Folder open, 408 images | 34 ms |
| Thumbnail worker time | p50 1 ms, p95 4 ms, max 293 ms |
| Memory (RSS) | 243 MB at start, 2,345 MB peak, 1,382 MB at end |

## Waiting On The User

- **Phase 3:** relevance judged on October 13. Gemma passed 30/30 and was chosen (D22); see `ROADMAP.md`. Still open: the indexing-impact check and the llama.cpp packaging check (Phase 3, items 4–5). Then come Phase 4 (fingerprints) and Phase 5 (visual search, tag retirement).
- **Thumbnail memory:** keep whole-folder preload, or cap it. Decide after measuring the deployed build (`ROADMAP.md` Phase 2, item 8).
- **Phase 4 (fingerprint-keyed buckets):** starts only after Phases 1–2 are accepted, because it migrates bucket data.
- **Independent audit (October 13):** `docs/archive/audit-2026-10-13.md` (Codex `gpt-6.1-sol`, 12 findings).
  - Findings 1–6 and 8–12 are fixed and deployed in `e9c142f`. The judging sheet was rebuilt, and the scorer now needs `--manifest`.
  - Finding 7 (thumbnail generations) is in the Backlog.

## Guardrails

- Test in isolated directories (`docs/reference/development.md`); never touch the live data directory.
- Commit each unit. No push, merge into `main`, or deployment without approval.
