# Phase 4.1 class cleanup and plot groups plan

1. Remove Validation from the runtime surface: navigation, frontend state/actions, Tauri commands,
   validation store/model/module, dedicated simulator paths, acceptance harness, and tests.
   Retain only optional legacy metadata deserialization so previously recorded files continue to
   open without being assigned a current validation status.
2. Keep profile integrity and Student/Instructor profile authoring intact, but remove all
   profile-validation badges, evidence associations, and validation-specific diagnostics.
3. Replace the Overlay/Stacked state with a current-session plot-group model. Each active signal
   is assigned exactly once to a numbered group; visibility remains independent from assignment.
   Empty groups have configuration slots but never create zero-height uPlot instances.
4. Add profile-specific default group assignments, reassignment selectors, plot-count controls,
   and Overlay all / One plot per signal presets. All groups reuse the same bounded display
   snapshot and raw recording path.
5. Update focused unit tests, documentation, simulator soak evidence, and manual acceptance logs.
   Preserve the existing root-level board cache, non-mutating startup verification, and operation
   modal behavior.
