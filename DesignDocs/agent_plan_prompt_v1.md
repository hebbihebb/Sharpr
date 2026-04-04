Read the attached design document in the folder DesignDocs carefully and use it as the primary architectural reference for this project.

I want you to create a phased implementation plan for building this Linux desktop application. The target app is a modern GTK4 + Libadwaita image library / preview application with future AI upscaling support, based on the design document and the agreed UI concept.

Important planning constraints:

- Treat the design document as the source of truth for the intended architecture, UX direction, and technical stack.
- The app should be built in phases, not as one giant implementation.
- The first deliverable is a functional desktop application with the agreed three-pane UI and core browsing/preview behavior.
- The second major deliverable is AI upscaling integration.
- The final deliverable is Flatpak packaging and a polished first-run permissions strategy.
- Even though Flatpak packaging is the final phase, you must account for sandboxing, file access, subprocess execution, and permissions from the beginning so the architecture does not paint us into a corner.

Assume these broad product goals:

- Linux-first desktop app
- GTK4 + Libadwaita UI
- Rust preferred unless the design document strongly implies otherwise
- Large central image preview
- Folder/navigation sidebar
- Thumbnail filmstrip
- Metadata overlay
- Good performance with large image collections
- Later support for AI upscaling through open-source tools invoked locally
- Final shipping target should be Flatpak-compatible

What I want from you in the planning phase:

1. Read and summarize the design document into a practical engineering interpretation.
2. Identify which parts are immediately implementable, which parts are medium risk, and which parts are stretch/polish features.
3. Convert the proposal into an MVP-first roadmap.
4. Propose a clean project architecture that an agent can implement incrementally.
5. Recommend the best crate/library/tool choices for each subsystem.
6. Call out any places where the design document is too ambitious, vague, or likely to cause implementation trouble.
7. Suggest adjustments that preserve the product vision while increasing the odds of successful agentic implementation.
8. Produce a concrete implementation plan with phases, milestones, dependencies, and acceptance criteria.

Very important implementation philosophy:

- Favor a working, clean, incremental architecture over trying to build every advanced feature immediately.
- Do not collapse everything into one large app module.
- Prefer small modules, explicit state boundaries, and a maintainable project layout.
- Performance matters. Avoid designs that will obviously become slow or memory-heavy with larger image libraries.
- The UI should closely follow the design intent, but the code should privilege robust GTK/Libadwaita patterns over fake mockup behavior.
- If a feature is high risk, propose a lower-risk version for MVP and label the higher-polish version as a later phase.

Please structure your output like this:

A. Product interpretation
- What the app is
- Core user flows
- Core technical assumptions

B. Risk review
- Low-risk features
- Medium-risk features
- High-risk features
- Anything in the design doc that should be deferred

C. Recommended architecture
- Language
- UI framework
- Data model
- Image loading / thumbnailing approach
- Metadata approach
- Upscaling integration approach
- State management approach
- Packaging considerations

D. MVP definition
- Exactly what is included
- Exactly what is excluded

E. Phased build plan
For each phase include:
- objective
- concrete tasks
- files/modules likely involved
- acceptance criteria
- likely pitfalls

F. Flatpak-forward considerations
Even though packaging is later, include:
- how filesystem access should be handled
- how external/local AI upscaling tools should be handled
- whether the app should rely on portals, bundled tools, host tools, or user guidance
- what first-run UX should do if permissions or required binaries are missing

G. Recommended first coding milestone
- the first practical implementation step to begin with
- what should be scaffolded before deeper features are attempted

Do not write code yet unless needed for a tiny example. Focus on producing the best possible implementation plan for an agentic workflow.

Be opinionated and practical. If the design document suggests something fragile or overcomplicated, say so and recommend a better path.

The goal is not just to admire the design. The goal is to successfully build it.

Additional constraints:

- Do not propose rewriting the product into a web app.
- Do not substitute Electron, Tauri, or Qt unless you can justify it extremely strongly against the design document.
- Keep the plan aligned with GTK4 + Libadwaita unless there is a severe implementation blocker.
- Assume the UI concept is agreed and should be approximated faithfully in native desktop patterns.
- Prefer open-source libraries and local tools.
- When uncertain, choose the path with the best maintainability and best compatibility with agentic coding tools.

At the end, give me:
1. the recommended MVP feature list,
2. the recommended project folder structure,
3. the recommended first 3 implementation tasks,
4. the biggest 5 risks that could derail the project.

Flatpak is a final deliverable, not the first milestone. However, the plan must account for Flatpak constraints from the beginning.

Assume the final app should either:
- use portals and sandbox-friendly behavior where possible,
- detect missing permissions or required tools at first run,
- and present a clear guided setup flow to the user if extra permissions, binaries, or overrides are needed.

Do not ignore packaging realities during architecture planning.
