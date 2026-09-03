# Prototype verdict

This branch is a throwaway. It holds the complete `computed-proto` Rust prototype
(the `demo`, `run`, `check`, `watch`, `cat` commands and the free-play actions in
`src/main.rs`) plus the two HTML logic prototypes under `prototypes/`, exactly as they
were when they answered their question. `main` keeps only the validated decisions
(ADR 0002 and later) and the v0 implementation. Do not merge this branch.

Captured under issue #3.

## Question

Can a marker-delimited region in a markdown file decide its own state, per region,
with nothing stored outside the file, and can a watch loop over such files stay quiet?

## Verdict

Yes. The two-sum closer, `sum=` for what the region was computed from and `out=` for
what the tool wrote, makes every per-region decision derivable from the files alone,
with no sidecar state:

- fresh: `sum=` matches the current inputs and `out=` matches the body
- stale: `sum=` differs from the current inputs
- hand-edited: `out=` differs from the body
- first render: no closer sums yet

The watch loop stays quiet in practice via settle, the own-write guard (`out=`
recognises the tool's own write coming back), and single-flight render.

## Known wart

A volatile region with no declared inputs, for example `sh cmd=date`, rewrites only
when its output actually changes. Same-second repeats are absorbed by `out=` plus
skip-if-unchanged. Across a second boundary `check` exits 1 and `run` rewrites.

## Running it

    cargo run -- demo   # recreates the scratch demo repo under .scratch/ (gitignored)
    cargo run -- run
    cargo run -- check
    cargo run -- watch

See `README.md` for the free-play actions and the scenario list.
