---
status: accepted
---

# Render is pure behind a `Loaders` seam

`render::file` takes a parsed template, a `Mode`, a trust flag and a `&mut dyn Loaders`, and returns a `Rendered` value: the new text and a report per region, or a refusal, or a file-level error. It performs no I/O. The `Loaders` trait has two methods, `snapshot` and `load`, mirroring the two loader steps fixed by ADR 0006; the `loader::Loader` enum is the production adapter and tests supply a table-driven fake. Every rule that decides what a region becomes lives in `render`: the freshness cache, the refuse-on-edit policy and its per-file consequence, the untrusted skip, loader failure keeping the old body, and `clean`. The cli only turns a `Rendered` into a write and an exit tier.

Ticket 03 ruled out a loader trait until a third loader arrives. That rule stands for the loader set, which is a closed enum. `Loaders` is a different thing: a seam for testing, and it has two adapters from the first commit, which is the bar for a seam being real rather than hypothetical.

## Considered Options

- **`render` calls loaders directly** (the prototype). Rejected: every render test runs a shell or walks a directory, and the cache and refuse rules can only be tested end to end.
- **Two pure calls, `states` then `apply`, with the cli running loaders between them.** Rejected: the rule that says which regions load (stale, unrendered, volatile, and not exec when untrusted) leaks into the cli, the one module without unit tests.
- **One pure call behind a two-method `Loaders` trait.** Chosen. Cost: one trait and one fake, and `render` must be handed the trust flag rather than discovering it.

## Consequences

- The sum computation is private to `render`; `marker` exposes the canonical opener and `loader` the format constant, and golden tests at `render`'s interface fix the sum vectors.
- Loader errors carry their exit tier: `LoadError::Hard` (tier 2, file skipped whole) and `LoadError::Failed` (tier 1, body kept). `render` maps both into `Rendered`.
- Fresh regions are reproduced from the raw marker lines `marker` keeps, so byte-for-byte preservation is copying, not re-serialising.
