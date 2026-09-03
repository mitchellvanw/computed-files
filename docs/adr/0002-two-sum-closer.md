---
status: accepted
---

# Two sums in the closer

Every region closer carries two sums, `in=` and `out=`: the input sum records what the body was computed from, the output sum records what the tool wrote. cog stores one checksum over the output only, which detects hand edits but says nothing about whether the inputs have moved; freshness then needs a side store, which the logic prototype showed is the first thing to go wrong in copy mode. With both sums in the file itself, `check` answers "did the inputs change" and "did someone edit the body" from the rendered file alone, with no cache directory and nothing to lose on clone.

## Considered Options

- **One sum over the output** (cog). Rejected: cannot tell drift from changed inputs without re-running every loader, which is the expensive path and, for `exec`, the untrusted one.
- **Sums in a sidecar file.** Rejected: a second file to commit, easy to lose, and the rendered file no longer explains itself.
- **Two sums in the closer.** Chosen. Cost: a longer closer line and a marker format that is harder to change once files carry it.
