---
status: accepted
---

# In-place is the only layout in v0

The prior-art research recommended copy mode, a `.tmpl` template rendered to the canonical path, because the ecosystem does that and it keeps a stable "this is generated" signal. The logic prototype then showed that copy mode reverts every prose edit made to the rendered file on the next run. For the dogfood target, `CLAUDE.md`, prose edits by agents and people are the normal case, so that revert is data loss, not a mode. v0 is in-place only: the template and the rendered file are one file, the tool owns region bodies and nothing else, and the two sums in the closer (ADR 0002) detect edits inside a region without a side store. The parser, render and sum core stay layout-agnostic so copy can return later as an opt-in.

## Considered Options

- **Copy as default** (research recommendation). Rejected: loses prose hand edits on agent-edited files, and needs an `adopt` step to port them back that is a merge tool in disguise.
- **Both layouts, `.tmpl` sibling means copy.** Rejected for v0: adds a file-discovery rule before file discovery is decided, and no v0 user needs it.
- **In-place only.** Chosen. Cost: markers and executable text live in a shared committed file, and a future `watch` must ignore its own writes.

## Consequences

- No file-level generated banner. The per-region opener suffix `| do not edit; run computed` is the only signal, and it is true at the granularity it claims.
- Writes go through a temp file in the same directory and `rename(2)`, with the original's mode bits copied. Nothing is written when the rendered bytes equal the current bytes.
- The `adopt` action and the copy-mode banner line are dead; the hand-edit policy only has to answer for region bodies.
