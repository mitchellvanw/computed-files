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
<!-- /computed in=763d3939163eb7ab70fd52696d3301d85cc291f1c44ba9e28ea64a43f8609c0f out=7197482b2f16496ca38acd1bb78a60ce7709f172c003ad2a11085ba982c71e8b -->

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
# Sums are full SHA-256

<!-- /computed in=f2d430a251fd8f0ba9f00ceafac63462e31365aa27a38ad983b812c3be446fc3 out=2165d202039a50eb9b98ef71e709644c20cec0dae66c8664c76a258de7a63585 -->
