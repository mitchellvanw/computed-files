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
├── action.yml
├── claude-code-plugin
│   ├── hooks
│   └── skills
├── docs
│   ├── adr
│   ├── catalogue.html
│   ├── how-it-works.html
│   ├── index.html
│   ├── regions.js
│   ├── research
│   └── spec
├── prototypes
│   ├── computed-markdown.prototype.html
│   └── hand-edit.prototype.html
├── scripts
│   ├── action
│   ├── adr-index.sh
│   ├── cli-commands.sh
│   └── site-install.sh
├── src
│   ├── adopt.rs
│   ├── affected.rs
│   ├── allow.rs
│   ├── cli.rs
│   ├── config.rs
│   ├── doctor.rs
│   ├── dupes.rs
│   ├── fs.rs
│   ├── git.rs
│   ├── graph.rs
│   ├── guard.rs
│   ├── index.rs
│   ├── launch.rs
│   ├── lib.rs
│   ├── loader.rs
│   ├── lsp.rs
│   ├── main.rs
│   ├── marker.rs
│   ├── merge.rs
│   ├── project.rs
│   ├── remote.rs
│   ├── render.rs
│   ├── report.rs
│   ├── sandbox.rs
│   ├── sink.rs
│   ├── stats.rs
│   ├── survey.rs
│   ├── symbol.rs
│   ├── table.rs
│   ├── toc.rs
│   ├── trace.rs
│   ├── transcript.rs
│   ├── truncate.rs
│   ├── trust.rs
│   ├── update.rs
│   ├── watch.rs
│   └── why.rs
└── tests
    ├── adopt.rs
    ├── affected.rs
    ├── cli.rs
    ├── doctor.rs
    ├── dupes.rs
    ├── exec_sandbox.rs
    ├── file_slice.rs
    ├── fixtures
    ├── guard.rs
    ├── history.rs
    ├── index.rs
    ├── lsp.rs
    ├── max_lines.rs
    ├── merge.rs
    ├── on_stale.rs
    ├── projected_inputs.rs
    ├── recipes.rs
    ├── remote.rs
    ├── render.rs
    ├── sandbox
    ├── stats.rs
    ├── symbol.rs
    ├── table.rs
    ├── toc.rs
    ├── trace.rs
    ├── transcript.rs
    ├── value.rs
    ├── watch.rs
    └── why.rs
```
<!-- /computed in=34b6b3659887aaac05bd9087ad0ec4f7d58ac8d26038cf3688c7cf0726ca43d5 out=51f71397645e174c40e2b360d7caaa80ba30219c387f8b00e2ea0d3954edff90 -->

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
- [Wildcards in `inputs=` do not reach ignored paths](docs/adr/0012-wildcards-in-inputs-do-not-reach-ignored-paths.md)
- [A region the tool cannot answer skips only itself](docs/adr/0013-a-region-the-tool-cannot-answer-skips-only-itself.md)
- [Snapshots ignore closer sums, and `run` settles templates that read each other](docs/adr/0014-snapshots-ignore-sums-and-run-settles-across-files.md)
- [The `file` loader](docs/adr/0015-the-file-loader.md)

<!-- /computed in=7889c53daa593b8da7ee057aaefb8a89a5b0ffe62cbe79d0df3496843c3a0765 out=1d5d18700a303ecb8aa4e56252ba741d3b04a292cec7d258a2712448d332b8b2 -->
