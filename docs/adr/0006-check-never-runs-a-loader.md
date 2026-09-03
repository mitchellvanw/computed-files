---
status: accepted
---

# `check` compares sums and never runs a loader

`check` decides a region's state from the rendered file and the inputs alone: it recomputes the input snapshot, compares its sum to `in=`, hashes the body and compares it to `out=`. It never executes a loader. `run` uses the same comparison as a cache and skips the loader of any region whose two sums match. This fixes the loader shape: a snapshot step that costs nothing dangerous and a load step that may run a shell command, with the snapshot computable before the load.

The alternative, rendering in memory and diffing, is what the logic prototype did and what cog's `--check` does. It gives `check` a diff to print, but it means every `check` runs every `exec` command, which is the slow path in a pre-commit hook and the unsafe path on a clone nobody has vetted. With two sums in the closer, the diff is not needed to answer "is anything stale or edited"; when someone wants to see it, `run --dry-run` renders and prints without writing.

## Considered Options

- **`check` renders and diffs** (cog, the prototype). Rejected: runs untrusted commands to answer a yes/no question, and pays the full loader cost on every commit.
- **`check` compares sums, `run` always renders.** Rejected: declaring `inputs=` would then buy nothing at `run` time, and the pre-commit hook still pays for every query.
- **`check` compares sums, `run` skips fresh regions.** Chosen. Cost: `check` cannot show the pending diff, and a loader whose output changes without an input or format-constant change is invisible until `--force`.
