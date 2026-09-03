# computed

`computed` keeps marked regions of a markdown file current by computation. This file is the dogfood target: the two regions below are owned by the tool, everything else is hand-written.

Read `CONTEXT.md` for the vocabulary and `docs/spec/computed-v0.md` for the design. Decisions that are hard to reverse live under `docs/adr/`.

## Working here

- `cargo test` runs everything. The `render` golden files under `tests/fixtures` pin the sum vectors; regenerate them with `UPDATE_GOLDEN=1 cargo test --test render` only when a rendering change is intended.
- `cargo clippy --all-targets` and `cargo fmt` must be clean.
- Do not edit the bodies of the regions below. `computed run` in pre-commit refuses a hand-edited region; `computed run --force` overwrites it.

## Layout

<!-- computed tree src=. depth=2 name=layout | do not edit; run computed -->
```
.
├── CLAUDE.md
├── CONTEXT.md
├── Cargo.lock
├── Cargo.toml
├── README.md
├── docs
│   ├── adr
│   ├── computed.html
│   ├── research
│   └── spec
├── prototypes
│   ├── computed-markdown.prototype.html
│   └── hand-edit.prototype.html
├── src
│   ├── cli.rs
│   ├── fs.rs
│   ├── lib.rs
│   ├── loader.rs
│   ├── main.rs
│   ├── marker.rs
│   ├── render.rs
│   ├── report.rs
│   ├── sink.rs
│   └── trust.rs
└── tests
    ├── cli.rs
    ├── fixtures
    └── render.rs
```
<!-- /computed in=50bdd8f3ff226f1e out=0dc59481e21c9200 -->

## Decisions

<!-- computed exec cmd="grep -h '^# ' docs/adr/*.md" inputs=docs/adr/*.md name=adrs | do not edit; run computed -->

# Rust for the prototype
# Two sums in the closer
# In-place is the only layout in v0
# Relative paths resolve against the template's directory
# `run` refuses a hand-edited region
# `check` compares sums and never runs a loader
# Exec trust is granted per clone, outside the working tree
# Render is pure behind a `Loaders` seam
# Loader text is normalised and exec runs in a pinned environment

<!-- /computed in=c48c1171d6c901ec out=18ff75c06e6e8068 -->
