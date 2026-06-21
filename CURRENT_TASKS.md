# Current Tasks

This file is the first handoff point after `AGENTS.md`. Keep it short and current.

## Current State

- Local `main` is ahead of `origin/main` with recent Tasks queue and planning commits.
- Last completed feature work: Tasks queue batch selection and bulk edit.
- Last completed planning work: reset project documentation to a tracked Codex-first Markdown base.
- Local Codex skills were updated to follow the new docs, including a new Sharpr agenda-loop skill.

## Next Recommended Work

1. Commit this task-state update if it is part of the skill-sync task.
2. Start roadmap step 18a: migrate tags and collections toward id-backed keys before image-family promotion work.

## Active Roadmap Focus

The next feature track is external edit workflow and image families:

- image families and versions
- attach externally edited versions
- managed Sharpr Edits storage
- Version Compare
- promotion to folder original
- Tasks integration with Create Version vs Export Copy

See `ROADMAP.md` and `docs/features/external-edit-workflow.md`.

## Open Decisions

- Step 18 should treat id-backed tags/collections as a prerequisite, even though an older design note said path-backed tags/collections were acceptable for MVP.
- Any implementation that touches version storage must preserve the non-destructive rule and disabled-folder scan exclusions.

## Handoff Checklist

- Update this file when a task starts, completes, or changes direction.
- Keep detailed historical notes out of this file; use `docs/archive/`.
- Before handing code work back, run `cd sharpr && cargo build`.
