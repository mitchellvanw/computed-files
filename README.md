# computed — PROTOTYPE (throwaway)

> **PROTOTYPE — throwaway code answering one question.** Not the real tool. When the question is
> answered, this tree moves to a throwaway branch and main keeps only the validated decision.

## The question

The in-memory logic prototype (`prototypes/computed-markdown.prototype.html`) validated the model,
but it kept the tool's own bookkeeping — a hash of what it last wrote, per region — as **runtime
state**. A real invocation of `computed` has no memory between runs. So:

**Can every decision — fresh vs stale, hand-edited, prose drift, first render — be derived from the
files alone, if the closer carries both sums: `sum=` (what the region was computed from) and `out=`
(what the tool wrote)?** And does the watch loop stay quiet for real: settle, own-write guard,
single-flight render?

The two-sum closer is the hypothesis under test. The HTML prototype needed `writtenRegions` in
memory; this prototype stores the output sum in the file itself and sees whether statelessness holds.

## Run it

    cargo run -- demo        # recreate the scratch demo repo (.scratch/computed-markdown-proto — wipe me)
    cargo run -- run         # render CLAUDE.md.tmpl → CLAUDE.md, atomic publish, skip if unchanged
    cargo run -- check       # CI mode: exit 1 on drift, never writes
    cargo run -- watch       # poll → settle → own-write guard → single-flight render
    cargo run -- cat         # print the template and the rendered file

Free-play actions mutate the demo repo the way someone else would; then `run`/`check` show the model
react: `add-file`, `del-file`, `add-row`, `rm-csv`, `edit-region [name]`, `edit-prose`, `add-sh`,
`trust`, `untrust`, `clean`. `--root <dir>` points the tool elsewhere.

## Module map (the liftable part)

    src/parse.rs     marker grammar → segments (pure)
    src/load.rs      loaders: tree · csv · sh (snapshot = what the input sum is taken over)
    src/sink.rs      sinks: table · fence · raw (pure)
    src/render.rs    region decisions, the two sums, whole-file render
    src/publish.rs   temp file + rename, skip if unchanged
    src/ops.rs       run_once / check_once — the entry points watch is built on
    src/watch.rs     poll loop: settle → own-write guard → single-flight render
    src/main.rs      throwaway CLI shell

## Scenarios (mirrors of the HTML prototype's walkthroughs)

    demo, run, add-file, check, run, check     a file appears (an mtime chain or generator hash misses it; the input sum doesn't)
    run, then watch the echo                   the tool's own write comes back → guard drops it
    run, run                                   nothing changed → write skipped, mtime untouched
    run, edit-region, run, run --force         hand edit → refusal, then forced discard
    run, edit-prose, run                       prose drift in copy mode → template wins, warned
    run, rm-csv, restore, run                  loader failure → last good region kept, exit 1, then repair
    add-sh, trust, run, run, check             volatile region: same sum, new body every run → churn (the known wart)
    run, clean, check, run                     fresh clone: missing file fails check, one run restores
