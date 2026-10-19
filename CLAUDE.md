# Claude Code Entry Point

Read `AGENTS.md` first; it is authoritative for every agent and lists what to read next. Keep this file limited to Claude-specific tool mechanics and never contradict `AGENTS.md`.

- A foreground `sleep` is blocked. Wait on long jobs with `run_in_background` or Monitor, not `sleep` and poll.
