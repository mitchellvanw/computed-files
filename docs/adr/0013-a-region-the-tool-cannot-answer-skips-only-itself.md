---
status: accepted
---

# A region the tool cannot answer skips only itself

A hard loader error, such as `src=` escaping the repository, `inputs=` matching nothing, or `/bin/sh` failing to start, takes its own region out of the render: the body and sums are kept, the region is reported `error` with the message beneath it, and the invocation exits 2. The other regions in the same file are checked, rendered and written as if the failing one were not there. A parse error still skips its file whole, because without a parse there are no regions to keep apart.

## Context

The spec made every tier-2 condition skip the file: "A tier-2 file is skipped whole; other files are still processed and written." For a parse error that is the only option. For a loader error it couples regions that share nothing but a file. A `CLAUDE.md` with a layout tree and an ADR index stops tracking its layout when someone deletes the last ADR, because `inputs=docs/adr/*.md` now matches nothing. `check` reports only the error, so CI cannot even say whether the tree is stale. The spec already keeps loader failures (tier 1) per region, with "the previous body is kept", so a hard error was the one outcome that did not.

## Considered Options

- **Keep skipping the file.** One rule for every tier-2 condition. Rejected: one broken region blinds the tool to every other region in the file, and the hook keeps failing on something unrelated to what the author just changed.
- **Downgrade loader hard errors to tier 1.** Treat them like a failing command. Rejected: tier 2 means the tool could not answer, and a glob matching nothing or a path escaping the repository is that. CI has to be able to tell "the content said no" apart from "the setup is wrong".
- **Skip the region, keep the tier.** Chosen. The exit code still says the setup is wrong, and the report names the region and why. Everything else in the file is still kept current.

## Consequences

- A file can be written by an invocation that exits 2. The exit code is the highest tier any region or file reached, as before. A written file is still reported region by region.
- `check` gains a state, `error`, for a region whose snapshot could not be taken. It counts as drift and makes the file tier 2.
- The report gains an action, `skipped; body kept`, for a region whose load hit a hard error under `run`.
- A hand-edited region still refuses the whole file. An errored region in a refused file is reported `error`, not `kept`.
