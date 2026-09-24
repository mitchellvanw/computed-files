---
status: accepted
---

# `on-stale=warn` softens only staleness

`on-stale=warn` is a common opener attribute, and `warn` is its only value. Under `check`, a region whose state is exactly `stale` and whose opener says `on-stale=warn` is reported with the action `warn`. It is printed without `-v` and does not raise the exit code. In JSON its `action` stays null and a `severity` key says `"warn"`. `edited`, `stale+edited`, `unrendered` and `error` fail as before. `run` is unchanged: the region re-renders like any stale region.

## Context

Some regions read inputs that move far more often than anyone wants CI to block on: a `git log` of the whole repository, a listing of a busy directory, a remote document someone else updates. The author had two choices. Declaring the inputs makes `check` fail on every change. `volatile` makes `check` pass without looking, so the region is never known to be out of date. Neither says "tell me, but don't stop the merge".

## Considered Options

- **A flag on `check`,** such as `--warn-stale`. No grammar change. Rejected: it applies to every region in the invocation, and the decision belongs to one region. In the opener it sits next to the region, travels with the file, and is part of the input sum.
- **`on-stale=ignore`,** silent. Rejected: that is `volatile` with extra steps. The point is that the staleness stays visible.
- **Soften every kind of drift for the region.** Rejected: `edited` is someone's hand edit, which `run` will refuse ([ADR 0005](0005-refuse-hand-edited-regions.md)). A warning would let the edit sit in the default branch while the next `run` fails. `unrendered` means the tool never wrote the body. Neither is the "inputs moved" case this is for.
- **Also skip the region under `run`.** Rejected: `run` is how the warning gets resolved. A stale region should render when someone runs.
- **Soften `stale` alone, under `check` alone.** Chosen.

## Consequences

- A warned region can stay stale in the default branch for as long as nobody runs `run`. The warning line is the only thing that says so.
- The attribute is part of the canonical opener, so adding or removing it makes the region stale once.
- Every JSON region object has a `severity` key, `null` unless softened, so the existing JSON shape gained a field. "`action` is null under `check`" still holds.
- A recipe may set `on-stale`, and a `use` region may override it.
