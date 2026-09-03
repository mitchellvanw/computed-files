# computed

Keep generated spans of a hand-written Markdown file current. The document is a view. The truth lives somewhere else: a directory listing, a CSV, the output of a command.

~~~markdown
## Layout

<!-- computed tree src=. depth=2 name=layout -->
```
.
├── CLAUDE.md.tmpl
├── data.csv
├── docs
│   └── notes.md
└── src
    ├── lib.rs
    └── main.rs
```
<!-- /computed sum=ce8c101b out=079d3f2c -->
~~~

The prose around the markers is yours. The body between them belongs to the tool. Run `computed run` and every region is recomputed; run `computed check` in CI and it exits 1 if anything drifted.

> **Status: prototype.** This tree is throwaway code written to answer one design question (below). The marker grammar, the two-sum closer, and the region decision table are the parts meant to survive. The CLI shell, the demo repo, and the polling watcher are not. Nothing here is published to crates.io.

## Why another region rewriter

Tools like cog, mdsh, and markdown-magic already rewrite the span between two comment markers. They all share one gap: they remember what they wrote, not what they read. cog stores a checksum over the output, so it can tell you a region was hand-edited. It cannot tell you the inputs moved. To answer "is this file fresh" they have to run every generator again, which is the slow path and, for shell regions, the untrusted one.

The dogfood target is `CLAUDE.md`. Agents and people edit that file constantly. It sits at the root of a repo whose layout changes every day. A file tree pasted into it is wrong within a week, and nobody notices, because nothing checks.

`computed` fixes both with one idea.

## The two-sum closer

Every closing marker carries two hashes:

```
<!-- /computed sum=2e155539 out=35e54bc1 -->
```

| Sum | Taken over | Answers |
|---|---|---|
| `sum=` | The opener line plus a snapshot of the region's declared inputs | Did the inputs change since this was written? |
| `out=` | The body the tool wrote | Did someone edit the body by hand? |

That is the whole state. There is no cache directory, no sidecar, no daemon memory. A fresh clone contains everything `check` needs, and a rendered file explains itself to anyone who opens it. [ADR 0002](docs/adr/0002-two-sum-closer.md) records the alternatives that lost.

The input sum is what separates this from an mtime chain. Creating a file in `src/` touches no file the region names. An mtime check misses it. A hash over the tree listing does not:

```
$ computed add-file           # someone creates src/watcher.rs; the template is untouched
$ computed check
check → exit 1
  layout     tree  stale      inputs changed; regenerated  sum=2e155539 out=35e54bc1
  data       csv   fresh      inputs unchanged             sum=42e56a14 out=a491e3d4
```

## What a region can be

Each region has one status after a render. Every status is derived from the files alone.

| Status | Meaning | `run` | `check` |
|---|---|---|---|
| `new` | No prior body. First render. | writes | exit 1 |
| `fresh` | Same input sum, same body. | skips the write | passes |
| `stale` | Input sum changed. Regenerated. | writes | exit 1 |
| `rewritten` | Same input sum, different body. The loader read something it did not declare. | writes | exit 1 |
| `edited` | Body no longer matches `out=`. Someone changed it by hand. | refuses, exit 1 | exit 1 |
| `error` | Loader failed. Last good body kept, sums kept. | keeps content, exit 1 | exit 1 |

Two of these deserve a note.

**Hand edits are refused, not silently reverted.** If the body's hash no longer matches `out=`, the tool leaves the region alone and exits 1 with a message. To regenerate, delete the sum from the closer or pass `--force`. This is the same policy cog uses and it exists for the same reason: an agent that edited a region had a reason, and clobbering it on the next run destroys work with no record.

**A loader failure keeps the last good body.** Deleting `data.csv` does not blank the table. The region stays as it was, the sums stay as they were, and the run exits 1 so CI notices. Restoring the file and running again repairs it.

## Loaders and sinks

A loader produces text and a snapshot of what it read. A sink shapes that text into what goes in the file.

| Loader | Attributes | Snapshot | Default sink |
|---|---|---|---|
| `tree` | `src=.` `depth=99` | The sorted list of visible paths | `fence` |
| `csv` | `src=file.csv` | The file's bytes | `table` |
| `sh` | `cmd="..."` `lang=` | None. The region is volatile. | `raw` |

Sinks: `fence` wraps the text in a code block, `table` renders rows as a Markdown table, `raw` writes the text as is. Any loader can name a sink with `sink=`.

Relative paths in a marker resolve against the directory of the file that contains the marker, not the repository root and not the shell's working directory. A region reads the same from a pre-commit hook, from CI, and from a terminal, and moving the file moves its regions with it. [ADR 0004](docs/adr/0004-region-root-is-the-template-directory.md) explains why the other two bases lose.

### Shell regions are off by default

`sh` runs arbitrary commands from a committed file. Cloning a repo should not execute anything, so the loader is disabled until a `.computed-trust` file exists in the region root. Until then the region reports an error and keeps whatever body it had.

A `sh` region declares no inputs, so its input sum never changes and its body might. Every run reports it as `rewritten`, every `check` fails, and the rendered file churns. This is the known wart of the current design. The fix under consideration is an `inputs=` glob on the opener so a command can say what it depends on, with `volatile` as the explicit opt-out.

## Layouts

The prototype renders a template to a sibling. `CLAUDE.md.tmpl` is what you edit; `CLAUDE.md` is what readers open, with a banner line on top. This is the copy layout.

It turned out to be the wrong default. In copy mode, any prose someone types into the rendered file is reverted on the next run, because the template wins. For `CLAUDE.md` that is the normal case, not an edge case, and reverting it is data loss. [ADR 0003](docs/adr/0003-in-place-layout.md) records the decision: the real tool is in-place only. Template and rendered file are one file, the tool owns region bodies and nothing else, and the two sums detect edits inside a region without a side store.

The parser, renderer, and sum logic are already layout-agnostic. The copy layout stays in this tree because the prototype's question was about statelessness, and copy mode is where a stateful design fails first.

## Writes

Every write goes through a temp file in the same directory followed by `rename(2)`. A reader never sees a half-written file. If the rendered bytes equal the current bytes, nothing is written and the mtime is untouched, so `run` twice in a row is a no-op and downstream watchers stay quiet.

## Watch

`computed watch` polls the region root, waits for the tree to settle, then renders once. Its own write comes back as a change event, so it remembers the last text it wrote and drops any event whose content matches. A change to the rendered file that does not match is someone else's edit; the watcher runs `check` on it rather than overwriting.

The loop is a 250 ms poll with a 150 ms settle window, written to prove the guard logic. The real tool will use the `notify` crate. Watch is a convenience layer; `run` in a pre-commit hook and `check` in CI are the foundation, which is why Rust won over Elixir for this. [ADR 0001](docs/adr/0001-rust-for-the-prototype.md) has the comparison.

## Try it

```
cargo run -- demo          # create the scratch repo under .scratch/ (wipes it)
cargo run -- run           # render CLAUDE.md.tmpl → CLAUDE.md
cargo run -- check         # exit 1 on drift, never writes
cargo run -- watch         # poll → settle → own-write guard → render
cargo run -- cat           # print both files
```

Then act like someone else in the repo and watch the model react:

```
add-file    del-file    add-row    rm-csv
edit-region [name]      edit-prose
add-sh      trust       untrust    clean
```

Scenarios worth walking through, each a mirror of a walkthrough in the HTML logic prototype:

| Sequence | What it shows |
|---|---|
| `demo` `run` `add-file` `check` `run` `check` | An undeclared new file. Only the input sum notices. |
| `run` then `watch`, then `run` in another shell | The tool's own write echoes back and the guard drops it. |
| `run` `run` | Nothing changed, so nothing is written. |
| `run` `edit-region` `run` `run --force` | Hand edit refused, then discarded on request. |
| `run` `edit-prose` `run` | Prose drift in copy mode. Template wins, with a warning. This is the failure ADR 0003 fixes. |
| `run` `rm-csv`, restore, `run` | Loader failure keeps the last good body and exits 1. |
| `add-sh` `trust` `run` `run` `check` | The volatile-region churn described above. |
| `run` `clean` `check` `run` | Fresh clone. A missing file fails `check`; one `run` restores it. |

## Layout of this repo

```
src/parse.rs      marker grammar → prose and region segments (pure)
src/load.rs       tree · csv · sh loaders, each returning text plus a snapshot
src/sink.rs       fence · table · raw (pure)
src/render.rs     region decisions, the two sums, whole-file render
src/publish.rs    temp file + rename, skip if unchanged
src/ops.rs        run_once and check_once, the entry points watch builds on
src/watch.rs      poll → settle → own-write guard → single-flight render
src/report.rs     the per-region report lines
src/main.rs       throwaway CLI shell and the demo repo

docs/adr/         the four decisions recorded so far
docs/research/    prior-art survey, gitignore semantics for the tree loader
prototypes/       the in-memory HTML logic prototypes this code was lifted from
CONTEXT.md        the vocabulary: template, region, marker, sum, snapshot, drift
```

## Gaps between the decisions and the code

The ADRs are ahead of the source. Anyone reading both should know:

- ADR 0002 names the input sum `in=`. The code writes `sum=`.
- ADR 0003 makes in-place the only layout. The code still renders `.tmpl` to a sibling.
- ADR 0004 names an `inputs=` attribute for exec regions and a `COMPUTED_ROOT` variable. Neither exists yet.
- The tree loader skips `.git` and `*.tmp` and nothing else. The real loader will honour `.gitignore` through the `ignore` crate; the semantics are worked out in `docs/research/ignore-gitignore-semantics.md`.
- Hashes are 32-bit FNV-1a. Fine for a prototype, not for a format files will carry.

## Vocabulary

The words in this README are chosen on purpose. A region, not a block. A marker, not a directive. A sum, not a checksum. Drift, not stale. `CONTEXT.md` lists each term with the words it replaces.
