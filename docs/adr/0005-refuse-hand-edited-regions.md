---
status: accepted
---

# `run` refuses a hand-edited region

When `run` finds a region whose body no longer matches its output sum, it writes nothing to that file, names the region, and exits 1. The user re-runs with `--force` to overwrite. Every surveyed tool except cog overwrites silently; cog refuses with "Output has been edited! Delete old checksum to unprotect." The hand-edit prototype showed why refusal is the only policy that holds: when a body edit and an input change land together, `overwrite` folds the edit into a legitimate re-render and the loss is indistinguishable from a normal run, and `warn` writes and exits 0, so on the pre-commit path it is overwrite with a line of output nobody reads. On the dogfood target, `CLAUDE.md`, the editor is usually an agent that does not know the region is owned, and refusal is the one signal that reaches it: the hook fails and the diff still shows what it wrote.

## Considered Options

- **Overwrite** (everyone but cog). Rejected: a hand edit that collides with an input change vanishes silently.
- **Overwrite with a warning.** Rejected: on the hook and CI path the warning is invisible and the exit code says success.
- **A policy switch** (`refuse | warn | overwrite`, as prototyped). Rejected for v0: `overwrite` is `--force` made permanent, `warn` has no user, and per-repo policy waits on the configuration decision.
- **Refuse, with `--force` to overwrite.** Chosen. Cost: a legitimate paste into a region blocks the commit until the user says `--force`, and a file with one edited region keeps its merely stale regions stale until then.

## Consequences

- Refusal is per file: a file with any edited region is not written at all, other files in the same invocation are, and the exit code is 1 if any file was refused.
- `--force` is unscoped; it overwrites every edited region in every file the invocation touches. Narrow by passing paths. `check` rejects `--force` as a usage error.
- No opener or closer edit reaches this policy: an edited opener is a template change and re-renders, a closer with both sums missing renders as never rendered, any other damaged closer is a parse error.
- `check` reports `edited` and `stale+edited` as distinct labels but the same exit code as `stale`; it answers one question, whether `run` would change or refuse the file.
