---
status: accepted
---

# Snapshots ignore closer sums, and `run` settles templates that read each other

When a snapshot reads a file, the `in=` and `out=` sums in that file's closer lines are taken out of the snapshotted content. And `run` does not stop after one pass over the files. It goes over every file whose snapshots read a file the pass just wrote, and repeats until a pass writes nothing. If files are still changing after one pass per file, they feed each other with no fixed point: those files are reported and the run exits 2.

## Context

`inputs=` can name another template. A docs index reads every `*.md`, and an `AGENTS.md` summary reads `CLAUDE.md`. Two defects followed:

- **Order.** Files are processed in byte order. When `README.md` reads `docs/guide.md` and both are stale, `README.md` renders first against the old guide. After `run` exits, `check` fails. Under pre-commit, every level of dependency costs a failed commit.
- **Cycles never settle.** Every write changes the file's closer sums. If `a.md` reads `b.md` and `b.md` reads `a.md`, then rendering `a.md` makes `b.md` stale, rendering `b.md` makes `a.md` stale, and so on forever, even when neither command's output changes. The only thing moving is the sums.

The sums are the tool's own bookkeeping. They move when the file renders, not when what it says changes, so they carry no information for the file that reads them.

## Considered Options

- **Document it.** Tell authors not to read templates. Rejected: an index of a docs directory is one of the first regions anyone writes.
- **Order files by a dependency graph.** Rejected because the graph is only known after globs expand, which is the snapshot step itself, and it still needs cycle handling.
- **Repeat until nothing is written, without touching snapshots.** This fixes order but not cycles, because the sums guarantee a cycle never settles.
- **Strip sums from snapshots, and repeat only the files that read a written file.** Chosen. With sums out of the snapshot, a cycle settles as soon as the outputs stop changing, and re-rendering only the affected files keeps a quiet pass cheap.

## Consequences

- One `run` leaves the files it touched fresh under `check`, whatever order they sort in.
- A region whose inputs include a rendered template gets a different input sum once, and `check` reports it stale until the next `run`. No format constant changes: a loader's output for the same inputs is unchanged, and it is the snapshot that moved. Regions whose inputs hold no closers are unaffected.
- Editing only the hex in a closer that sits in a fenced example no longer drifts a region that reads that file.
- A genuine feedback loop, where each render changes what the other reads, is a tier-2 error after one pass per file, not a hang.
- A region block reported identically by more than one pass is printed once. `--dry-run`, `check` and `clean` write nothing, so they make one pass.
- The `file` loader applies the same stripping to the text it includes ([ADR 0015](0015-the-file-loader.md)), so an included body never shows sums that no longer match.
