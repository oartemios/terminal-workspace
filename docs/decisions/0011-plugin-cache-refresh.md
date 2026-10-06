# 0011 — Plugin-owned cache with Core refresh scheduling

Status: implementation decision for MVP issue #5.

The plugin owns cached domain data and its persistence. Core owns only generic refresh strategy and task lifecycle. A plugin's `view` returns the current state and should serve cached data promptly when available; `refresh` updates that state and returns the refreshed view. Session-only cache is sufficient; plugins may persist their own cache when useful.

Plugin API 0.7 declares `Manual`, `OnFocus`, or `Interval { seconds }` per group. Event refresh is excluded until the public event mechanism in #9. Executable packages carry the declaration in their descriptor and receive a `refresh` operation over JSON-lines protocol 1. API 0.4–0.6 manifests remain compatible with `Manual` defaults.

The TUI loads the current view first, then starts automatic refresh when requested. Manual refresh is a registered `core.refresh` command, so `r`, command line, and palette share one route. A failed refresh preserves the last displayed Items and identifies them as stale. Stable Item IDs retain selection. Existing polling epochs and plugin lifecycle stop rules suppress late results and cancel work on context/lifecycle changes.

This contract does not require a Core cache schema, timestamp model, or disk persistence. Offline-specific error meaning is supplied by the plugin's error text; the generic UI marks retained data stale. OS process isolation remains outside this decision.

Validation target: independently packaged Catalog declares on-focus refresh and exercises the executable refresh route; runtime tests cover refresh success/failure, repeated requests, cancellation, and Workspace changes.
