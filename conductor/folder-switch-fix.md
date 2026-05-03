# Plan: Fix Folder-Switch Regression & Architectural Bottlenecks

## Objective
Eliminate UI hitches during folder switching and prevent thumbnail loading from breaking due to channel congestion when navigating between large folders.

## Key Files & Context
- `sharpr/src/thumbnails/worker.rs`: Background worker pool management and stale request handling.
- `sharpr/src/ui/filmstrip.rs`: Main-thread scheduling logic for thumbnail generation.
- `sharpr/src/ui/window.rs`: Orchestration of folder switches and generation bumps.

## Proposed Solution

### 1. Bounded Channels (Worker Back-pressure)
Switch from `async_channel::unbounded` to `async_channel::bounded(256)`.
- **Why**: Prevents the UI from dumping thousands of requests into a queue that might become stale immediately. If the queue is full, the UI will simply skip enqueuing (it will try again on the next scroll/event).
- **Impact**: Dramatically reduces memory pressure and stale request "lag" when switching folders.

### 2. Fast Stale Request Disposal (Worker Loop)
Optimize the worker loop to aggressively discard stale requests.
- **Why**: Currently, workers discard stale requests one-by-one, alternating with other logic.
- **Change**: When a stale request is detected, enter a tight `try_recv` loop to drain as many stale requests as possible from the channel before processing any "fresh" work.

### 3. Throttled/Incremental Scheduling (Filmstrip)
Optimize `schedule_visible_thumbnails` to reduce main-thread CPU usage.
- **Why**: Iterating over 1000+ entries on every scroll update is expensive.
- **Change**: 
    - Always scan and enqueue **visible** rows immediately.
    - Only scan the **buffer** (preloading) if the scroll position has changed significantly (e.g., > 200px) or if we are idle.
    - Reduce `BUFFER_ROWS` to `100` (down from `500`). This is still a generous ~200-300 images ahead but is much cheaper to scan.

### 4. Generation Sync Fix
Ensure the `pending_thumbnails` set is cleared and the generation is bumped atomically relative to the filmstrip's view of the model.

## Implementation Steps

1. **Setup**: Create a new branch `fix/folder-switch-performance`.
2. **Worker Update**: Modify `sharpr/src/thumbnails/worker.rs` to use bounded channels and implement the "tight drain loop" for stale requests.
3. **Filmstrip Update**: Modify `sharpr/src/ui/filmstrip.rs` to implement throttled buffer scanning and reduced constants.
4. **Validation**: Run existing tests and perform manual stress tests with rapid folder switching.

## Verification
- **Stress Test**: Switch between "Wallpapers" (1500 images) and "Art" (1200 images) every 1 second for 10 seconds.
- **Check**:
    - UI remains responsive (no hitches/freezes).
    - Thumbnails for the *current* folder start loading immediately.
    - Channel does not overflow with stale requests.
