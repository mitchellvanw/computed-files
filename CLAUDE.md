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
├── LICENSE-MIT
├── README.md
├── docs
│   ├── adr
│   ├── how-it-works.html
│   ├── index.html
│   ├── research
│   └── spec
├── prototypes
│   ├── computed-markdown.prototype.html
│   └── hand-edit.prototype.html
├── scripts
│   └── adr-index.sh
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
<!-- /computed in=92d6e0266a80e84b91816278be092562aeba9f8ada1a61a7dbc5be639aa0fc79 out=0ede9cf871428c78c25c4961bcab0b6ce8319121d0ec8782e0d1884380e42269 -->

## Decisions

<!-- computed exec cmd=scripts/adr-index.sh inputs=docs/adr/*.md,scripts/adr-index.sh name=adrs | do not edit; run computed -->

- [Rust for the prototype](docs/adr/0001-rust-for-the-prototype.md)
- [Two sums in the closer](docs/adr/0002-two-sum-closer.md)
- [In-place is the only layout in v0](docs/adr/0003-in-place-layout.md)
- [Relative paths resolve against the template's directory](docs/adr/0004-region-root-is-the-template-directory.md)
- [`run` refuses a hand-edited region](docs/adr/0005-refuse-hand-edited-regions.md)
- [`check` compares sums and never runs a loader](docs/adr/0006-check-never-runs-a-loader.md)
- [Exec trust is granted per clone, outside the working tree](docs/adr/0007-exec-trust-per-clone.md)
- [Render is pure behind a `Loaders` seam](docs/adr/0008-render-is-pure-behind-a-loaders-seam.md)
- [Loader text is normalised and exec runs in a pinned environment](docs/adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md)
- [Sums are full SHA-256](docs/adr/0010-sha-256-sums.md)
- [The tree loader honours `.gitignore` without a flag](docs/adr/0011-gitignore-is-not-a-flag.md)

<!-- /computed in=1e8d787e4c0352a4afd02b09b836d958f76e450cabc9a23620d5864908e4f3e9 out=dab14b7f7c1f04840e7bf094746072bf01bbf2b3acf25cbec5ee2569764f48f4 -->
