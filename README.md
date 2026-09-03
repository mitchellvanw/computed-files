# computed

Keep marked regions of a hand-written Markdown file current. The document is a view. The truth lives somewhere else: a directory listing or the output of a command.

~~~markdown
## Layout

<!-- computed tree src=. depth=2 name=layout | do not edit; run computed -->
```
.
├── CLAUDE.md
├── docs
│   └── adr
└── src
    ├── lib.rs
    └── main.rs
```
<!-- /computed in=9f3a1c0b7d2e4f60 out=41c0d9e8b3a2f715 -->
~~~

The prose around the markers is yours. The body between them belongs to the tool. `computed run` renders every region that needs it; `computed check` in CI exits 1 if anything drifted, without running a single command.

One static binary, Rust, no runtime to install. The full design is in [`docs/spec/computed-v0.md`](docs/spec/computed-v0.md); the decisions that are hard to reverse are under [`docs/adr/`](docs/adr/). This repository is its own first user: [`CLAUDE.md`](CLAUDE.md) carries a tree region and an exec region, kept current by `computed run` in pre-commit and verified by `computed check` in CI.

## Why another region rewriter

Tools like cog, mdsh, and markdown-magic already rewrite the span between two comment markers. They share one gap: they remember what they wrote, not what they read. cog stores a checksum over the output, so it can tell you a region was hand-edited. It cannot tell you the inputs moved. To answer "is this file fresh" they have to run every generator again, which is the slow path and, for shell regions, the untrusted one.

The dogfood target is `CLAUDE.md`. Agents and people edit that file constantly. It sits at the root of a repo whose layout changes every day. A file tree pasted into it is wrong within a week, and nobody notices, because nothing checks.

## The two-sum closer

Every closing marker carries two sums, the first 16 hex characters of a BLAKE3 hash each:

```
<!-- /computed in=9f3a1c0b7d2e4f60 out=41c0d9e8b3a2f715 -->
```

| Sum | Taken over | Answers |
|---|---|---|
| `in=` | The canonical opener plus a snapshot of the region's inputs | Did the inputs or the opener change since this was written? |
| `out=` | The body the tool wrote | Did someone edit the body by hand? |

That is the whole state. There is no cache directory, no sidecar, no daemon memory. A fresh clone contains everything `check` needs, and a rendered file explains itself to anyone who opens it. [ADR 0002](docs/adr/0002-two-sum-closer.md) records the alternatives that lost.

The input sum is what separates this from an mtime chain. Creating a file under `src/` touches nothing the region names. An mtime check misses it. A hash over the tree listing does not:

```
$ touch src/watcher.rs
$ computed check
CLAUDE.md:15 layout tree stale
$ echo $?
1
```

## What a region can be

Every state is derived from the file and its inputs alone.

| State | Meaning | `run` | `check` |
|---|---|---|---|
| `fresh` | Both sums match. | leaves it alone | passes |
| `stale` | Input sum differs. Inputs or the opener changed. | re-renders | exit 1 |
| `edited` | Body sum differs. Someone changed the body by hand. | refuses the file, exit 1 | exit 1 |
| `stale+edited` | Both. | refuses the file, exit 1 | exit 1 |
| `volatile` | Declares no inputs; body matches `out=`. | re-renders every time | passes |
| `unrendered` | The closer carries no sums. | renders | exit 1 |

**Hand edits are refused, not silently reverted.** A file with one edited region is left untouched in full and the run exits 1 with the region named. `run --force` overwrites. The reason is the case where a body edit and an input change land together: under overwrite the edit vanishes inside a legitimate re-render, and on `CLAUDE.md` the editor is usually an agent that does not know the region is owned. A failed hook is the one signal that reaches it. [ADR 0005](docs/adr/0005-refuse-hand-edited-regions.md).

**A loader failure keeps the last good body.** Non-zero exit, timeout, or output that fails normalisation: the body and sums stay as they were, the command's stderr is printed under the region's line, and the run exits 1 so CI notices. Restoring the input and running again repairs it.

**`check` never runs a loader.** It recomputes snapshots, compares both sums and reports. So it is safe on an unvetted clone and cheap in a hook. The diff `run` would write lives on `run --dry-run`. [ADR 0006](docs/adr/0006-check-never-runs-a-loader.md).

## Loaders and sinks

A loader produces text and a snapshot of what it read. A sink shapes that text into what goes in the file.

| Loader | Attributes | Snapshot | Default sink |
|---|---|---|---|
| `tree` | `src=.` `depth=` and the flags `all` `dirs` | One relative path per line, from the same walk that drew the listing | `fence` |
| `exec` | `cmd=` `timeout=30` and exactly one of `inputs=` or `volatile` | Path, length and content of every matched file; empty when volatile | `raw` |

Common attributes: `name=` for stable reports, `as=` to pick a sink, `lang=` for the fence language. The `tree` loader is gitignore-aware through the `ignore` crate, with the per-clone and per-user exclude files switched off so the listing is the same on every machine.

Relative paths in a marker resolve against the directory of the file that contains the marker, not the repository root and not the shell's working directory, and an exec command runs there. A region reads the same from a pre-commit hook, from CI, and from a terminal, and moving the file moves its regions with it. [ADR 0004](docs/adr/0004-region-root-is-the-template-directory.md).

Every loader's text is normalised before a sink sees it, and exec runs with `LC_ALL=C`, `TZ=UTC` and an empty `LANGUAGE`, so the same inputs give the same bytes on the developer's machine and in CI. [ADR 0009](docs/adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md).

### Exec regions run only in a trusted clone

Cloning a repository should not execute anything in it. An exec region runs only after `computed trust` has recorded a grant for that repository root on this machine, in `~/.config/computed/trust.toml`, never in the working tree. Until then `run` skips the region, keeps its body, reports it `untrusted` and exits 1. Tree regions in the same file still render. CI passes `run --trust` for one invocation; a `check`-only pipeline needs no trust at all. [ADR 0007](docs/adr/0007-exec-trust-per-clone.md).

## The command line

```
computed run   [paths] [--force] [--dry-run] [--trust]
computed check [paths]
computed clean [paths] [--force] [--dry-run]
computed trust   [path]
computed untrust [path]
```

With no paths, the current directory is walked with the tree loader's ignore settings and every `.md` file is read. An explicit file is read whatever its extension.

| Exit | Meaning |
|---|---|
| 0 | Nothing to report. Everything is fresh. |
| 1 | The content said no: drift under `check`, a write, a refused file, a loader failure or an untrusted region under `run`. |
| 2 | The tool could not answer: usage error, marker parse error, a path escaping the repository, `inputs=` matching nothing. |

One line per region goes to stderr; `--dry-run` diffs are the only thing on stdout. Fresh regions print only with `-v`.

### Hooks

Pre-commit runs `run`, CI runs `check`. The committed [`.pre-commit-config.yaml`](.pre-commit-config.yaml) sets `always_run` and `pass_filenames: false`, because a staged change under `src/` makes a region in `CLAUDE.md` stale without staging `CLAUDE.md`. Without the framework, `.git/hooks/pre-commit` is two lines:

```sh
#!/bin/sh
exec computed run
```

## Try it

```
cargo install --path .
computed trust          # once per clone
computed run            # renders CLAUDE.md, exits 1 because it wrote
computed run            # writes nothing, exits 0
computed check          # exits 0
```

Then act like someone else in the repository: add a file under `src/` and `check` exits 1; edit a line inside a region and `run` refuses; `clean` empties every region and `run` rebuilds it.

## Layout of this repo

```
src/marker.rs     the marker grammar: parse to prose and regions, serialise back
src/sink.rs       normalisation, raw and fence
src/render.rs     the sums, the states, refuse, untrusted, failure, clean; pure behind a Loaders seam
src/fs.rs         the walk, the repository root, the atomic write
src/loader.rs     tree over the walk, exec over /bin/sh with the pinned environment
src/trust.rs      the per-clone grant store
src/report.rs     the stderr line per region and the dry-run diff
src/cli.rs        the five commands, discovery, exit tiers

tests/render.rs   render through a fake Loaders against golden files; the goldens pin the sum vectors
tests/cli.rs      the pre-commit scenario and the exit tiers, end to end

docs/spec/        the v0 specification
docs/adr/         the decisions, argued
docs/research/    prior-art survey, gitignore semantics for the tree loader
prototypes/       the HTML logic prototypes the design was tested on
CONTEXT.md        the vocabulary: template, region, marker, sum, snapshot, drift
```

`render` is pure: it takes a parsed file, a mode, a trust flag and a `Loaders` seam and returns what to write and a report per region. The production loaders and a table-driven fake both sit behind the seam, so every rule about what a region becomes is tested without a shell or a directory walk. [ADR 0008](docs/adr/0008-render-is-pure-behind-a-loaders-seam.md).

## Not yet

`watch` is deferred until dogfooding shows agent sessions reading stale regions between commits. Also not in v0: a copy layout, a `file` loader, sinks beyond `raw` and `fence`, configuration files, colour or machine-readable reports, Windows. The spec lists each with the reason it waits.

## Vocabulary

The words in this README are chosen on purpose. A region, not a block. A marker, not a directive. A sum, not a checksum. Drift, not stale, for a file as a whole. `CONTEXT.md` lists each term with the words it replaces.
