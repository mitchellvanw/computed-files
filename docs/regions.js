// The catalogue of regions. Loaded by both docs/catalogue.html, which lists
// them all, and docs/index.html, which shows one at random in the hero.
//
//   g   the group heading            n   the entry number
//   f   the file it would live in     who agent | human | both
//   l   tree | exec                   m   the marker line, real grammar
//   d   why it rots today             c   a caveat, when there is one
//   x   an example of the body        xk  live | real | shape
//
// xk says how far the example can be trusted: `live` is a region running in
// this repository, `real` is output produced from it, `shape` is illustrative.

(function () {
const O = "\x3C!-- computed ", C = " --\x3E";
const ENTRIES = [

{ g: "Agent-first", n: 1, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "Module → concept map, one line per file from its own top doc comment",
  m: O + "exec cmd=scripts/module-map.sh inputs=src/*.rs,scripts/module-map.sh name=modules as=fence" + C,
  d: `Two sources of truth for one fact. In this repository the README hand-copies the same descriptions that already exist as <code>//!</code> comments in the source, and the two wordings have already diverged.`,
  xk: "real",
  x: `cli.rs      The five commands: clap definitions, discovery, per-file context…
fs.rs       The walk, the repository root, and the atomic write.
loader.rs   The two loaders, tree and exec, and the production Loaders adapter.
marker.rs   The marker grammar: parse a file into prose and regions, and serialise it back.
render.rs   Decides what every region becomes. Pure: a parsed file, a mode, a trust…
report.rs   The stderr line per region, loader stderr indented beneath, and the…
sink.rs     The two sinks, raw and fence, and the normalisation every loader's…
trust.rs    The per-clone trust store: trust.toml under XDG_CONFIG_HOME, one…` },

{ g: "Agent-first", n: 2, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "The internal dependency graph — which module uses which",
  m: O + "exec cmd=scripts/module-graph.sh inputs=src/*.rs,scripts/module-graph.sh name=modgraph as=fence" + C,
  d: `Nobody maintains one, so an agent infers it by reading every file. Thirteen edges in this repository, derivable in one line of shell.`,
  xk: "real",
  x: `cli    -> loader, marker, render, trust
loader -> fs, marker, render
render -> loader, marker, sink
report -> render
sink   -> marker
trust  -> fs` },

{ g: "Agent-first", n: 3, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "The public API surface — every exported item",
  m: O + `exec cmd="cargo public-api --simplified" inputs=src/*.rs,Cargo.toml name=api as=fence` + C,
  d: `The most-hallucinated class of fact there is. An agent that cannot see the surface invents plausible function names.`,
  c: `<code>cargo public-api</code> needs a nightly toolchain to build rustdoc JSON, and its output ordering is not documented as stable.`,
  xk: "shape",
  x: `pub fn run(args: &Args) -> Result<ExitCode>
pub struct Region { pub name: Option<String>, pub line: usize }
pub enum State { Fresh, Stale, Edited, Volatile, Unrendered }
pub trait Loaders { fn tree(&self, m: &Marker) -> Result<Loaded>; }` },

{ g: "Agent-first", n: 4, f: "CHANGELOG.md", who: "agent", l: "exec",
  t: "The public API diff since the last tag",
  m: O + `exec cmd="cargo public-api diff latest" inputs=src/*.rs,Cargo.toml name=apidiff as=fence` + C,
  d: `Microsoft's api-extractor proves the value: the API report is tracked by git, a pull request that forgets to update it fails validation, and the file extension itself can gate stakeholder review. Almost nobody outside TypeScript does this.`,
  xk: "shape",
  x: `Removed items in this release
-pub fn render_all(files: &[PathBuf]) -> Result<()>

Added items in this release
+pub fn render(file: &File, mode: Mode) -> Rendered` },

{ g: "Agent-first", n: 5, f: "docs/errors.md", who: "agent", l: "exec",
  t: "Every error the binary can emit",
  m: O + "exec cmd=scripts/error-index.sh inputs=src/*.rs,scripts/error-index.sh name=errors as=fence" + C,
  d: `rustc generates its error index this way. curl's exit-code table is hand-maintained and admits its own drift. This repository has 28 distinct error sites in <code>marker.rs</code> alone, listed nowhere.`,
  xk: "real",
  x: `missing loader
unknown loader
attribute without a key
exec needs cmd=
exec needs inputs= or the volatile flag
exec takes inputs= or volatile, not both
unterminated marker: the line does not end with -->
…` },

{ g: "Agent-first", n: 6, f: "docs/errors.md", who: "agent", l: "exec",
  t: "The exit-code table, derived from the code rather than the prose",
  m: O + "exec cmd=scripts/exit-codes.sh inputs=src/cli.rs,scripts/exit-codes.sh name=exits as=fence" + C,
  d: `Measured here: the exit-code table exists in four hand-maintained copies and zero computed ones. git ships no exit-status table at all.`,
  xk: "shape",
  x: `| Exit | Meaning |
|---|---|
| 0 | Nothing to report. Everything is fresh. |
| 1 | The content said no. |
| 2 | The tool could not answer. |` },

{ g: "Agent-first", n: 7, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "The environment variables the code actually reads",
  m: O + "exec cmd=scripts/env-vars.sh inputs=src/*.rs,scripts/env-vars.sh name=env as=fence" + C,
  d: `Cargo's own environment reference is hand-written, with no tidy check that enumerates the <code>env::var</code> calls. An open cargo issue puts it plainly: “There's an env var for it already sitting in the code.”`,
  xk: "real",
  x: `HOME
UPDATE_GOLDEN
XDG_CONFIG_HOME` },

{ g: "Agent-first", n: 8, f: "README.md", who: "agent", l: "exec",
  t: "Feature flags and their defaults",
  m: O + "exec cmd=scripts/features.sh inputs=Cargo.toml,scripts/features.sh name=features as=fence" + C,
  d: `<code>cargo metadata --no-deps --format-version 1</code> hands you the map directly. docs.rs admits it has “only limited support for extracting structured feature metadata”, and the <code>document-features</code> crate exists solely for this problem.`,
  xk: "shape",
  x: `default = ["fence"]
fence   = []
json    = ["dep:serde_json"]
watch   = ["dep:notify"]` },

{ g: "Agent-first", n: 9, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "The test inventory — which test covers which module",
  m: O + `exec cmd="cargo test --quiet -- --list" inputs=src/*.rs,tests/*.rs name=tests as=fence` + C,
  d: `Tells an agent which test to run for a change instead of running everything or nothing. Fifty-six tests in this repository, listed nowhere. <code>cargo nextest list --message-format json</code> if you want structure.`,
  xk: "real",
  x: `fs::tests::repo_root_walks_up_for_git_and_canonicalises: test
fs::tests::walk_all_includes_dotfiles_but_never_the_git_directory_contents: test
fs::tests::walk_depth_counts_like_tree_and_dirs_lists_directories_only: test
loader::tests::exec_inputs_snapshot_lists_matched_files_with_their_content: test
loader::tests::tree_lists_and_snapshots_one_walk: test
render::tests::check_never_loads: test
…` },

{ g: "Agent-first", n: 10, f: "tests/README.md", who: "agent", l: "exec",
  t: "The fixture inventory — what each golden file pins",
  m: O + "exec cmd=scripts/fixtures.sh inputs=tests/fixtures/*,scripts/fixtures.sh name=fixtures as=fence" + C,
  d: `Thirty fixture files across three scenarios here, documented nowhere. An agent that does not know a fixture exists regenerates it wrongly, or writes a second one beside it.`,
  xk: "real",
  x: `fresh.check.report.txt
fresh.clean.txt
fresh.in.txt
fresh.stale-layout.txt
states.check.report.txt
states.dry-run.txt
states.force.txt
states.in.txt
…` },

{ g: "Agent-first", n: 11, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "Which decisions a given file implements — the traceability ledger, inverted",
  m: O + "exec cmd=scripts/adr-map.sh inputs=src/*.rs,docs/adr/*.md,scripts/adr-map.sh name=adrmap as=fence" + C,
  d: `Rust's tidy does the two-way version for error codes and errors in both directions. Nobody does it for design decisions. An index of ADRs tells you what was decided; this tells you where the decision lives.`,
  xk: "shape",
  x: `ADR 0004  src/loader.rs, src/marker.rs
ADR 0005  src/render.rs
ADR 0006  src/cli.rs, src/render.rs
ADR 0008  src/render.rs, tests/render.rs` },

{ g: "Agent-first", n: 12, f: "docs/invariants.md", who: "agent", l: "exec",
  t: "The invariants a module upholds, taken from its contracts",
  m: O + `exec cmd="cargo kani list --format markdown" inputs=src/*.rs name=invariants as=raw` + C,
  d: `<code>cargo kani list --format markdown</code> emits exactly this table today, cross-referencing every <code>#[requires]</code> and <code>#[ensures]</code> against its harnesses. Forty <code>assert!</code> sites in this repository, zero documented invariants.`,
  c: `<code>contracts</code> and <code>creusot</code> have the attributes but no listing tool; Prusti is unmaintained.`,
  xk: "shape",
  x: `| Harness | Contract | Status |
|---|---|---|
| verify_sum_roundtrip | marker::serialise | verified |
| verify_body_never_holds_a_marker | sink::normalise | verified |` },

{ g: "Agent-first", n: 13, f: "CONTRIBUTING.md", who: "agent", l: "exec",
  t: "What a fresh clone assumes is installed",
  m: O + "exec cmd=scripts/prereqs.sh inputs=rust-toolchain.toml,Cargo.toml,.github/workflows/ci.yml,scripts/prereqs.sh name=prereqs as=fence" + C,
  d: `Present in six of eighteen surveyed context files, and always written by hand. Delegating it — “version pinned in <code>.nvmrc</code>” — costs the agent a round trip it usually will not take.`,
  xk: "shape",
  x: `rust      1.90, stable channel
git       2.39 or newer
sh        POSIX
optional  pre-commit, for the hook` },

{ g: "Agent-first", n: 14, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "The exact commands CI runs",
  m: O + "exec cmd=scripts/ci-steps.sh inputs=.github/workflows/ci.yml,scripts/ci-steps.sh name=ci as=fence" + C,
  d: `Five commands here, and no document lists them. An agent that knows the five checks can run them before claiming it is done, which is the difference between a green branch and a review comment.`,
  xk: "real",
  x: `uses: actions/checkout@v4
uses: dtolnay/rust-toolchain@stable
run:  cargo build --release
run:  cargo test
run:  cargo clippy --all-targets -- -D warnings
run:  cargo fmt --check
run:  computed check . docs/index.html` },

{ g: "Agent-first", n: 15, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "The per-directory ownership map",
  m: O + `exec cmd="cat ../.github/CODEOWNERS" inputs=../.github/CODEOWNERS name=owners as=fence` + C,
  d: `CODEOWNERS is already a per-path ownership map, and the table in the docs duplicates it. <code>codeowners-generator --preserve-block-position</code> exists precisely to keep a generated block inside a hand-editable file — the same idea, one file over.`,
  xk: "shape",
  x: `/src/loader.rs   @team-core
/src/trust.rs    @team-security
/docs/adr/       @team-core @maintainers` },

{ g: "Agent-first", n: 16, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "The gitignored surface — what the agent will never see",
  m: O + `exec cmd="git status --ignored --porcelain | sed -n 's/^!! //p'" volatile name=hidden as=fence` + C,
  d: `Never documented anywhere, and the source of a whole class of confused agent behaviour: looking for a build product that is not in the tree.`,
  c: `<code>volatile</code>, so <code>check</code> always passes it. A convenience region, not a checked one.`,
  xk: "real",
  x: `.DS_Store
.scratch/
target/` },

{ g: "Agent-first", n: 17, f: "CLAUDE.md", who: "agent", l: "exec",
  t: "Repository size calibration — lines per module, file counts",
  m: O + `exec cmd="wc -l src/*.rs" inputs=src/*.rs name=sizes as=fence` + C,
  d: `Copilot's documentation explicitly asks for “the size of the repo”, and every real context file either writes it by hand or leaves it out. It is what tells an agent whether to read a file or outline it.`,
  xk: "real",
  x: `     234 src/cli.rs
     255 src/fs.rs
      16 src/lib.rs
     619 src/loader.rs
       3 src/main.rs
     865 src/marker.rs
     485 src/render.rs
      80 src/report.rs
     169 src/sink.rs
     180 src/trust.rs
    2906 total` },

{ g: "Human-first", n: 18, f: "README.md", who: "human", l: "exec",
  t: "Direct dependencies, with versions",
  m: O + `exec cmd="cargo tree --depth 1 -e no-dev" inputs=Cargo.toml,Cargo.lock name=deps as=fence` + C,
  d: `This repository's spec hand-lists eleven crates with no versions — already a second source of truth beside <code>Cargo.lock</code>. <code>cargo tree</code>'s box-drawing and <code>(*)</code> markers are stable output.`,
  xk: "real",
  x: `computed v0.1.0
├── anyhow v1.0.104
├── clap v4.6.6
├── globset v0.4.20
├── ignore v0.4.33
├── libc v0.2.189
├── serde v1.0.229
├── sha2 v0.11.0
├── similar v3.2.0
├── tempfile v3.27.0
├── toml v1.1.5+spec-1.1.0
└── wait-timeout v0.2.1` },

{ g: "Human-first", n: 19, f: "docs/licences.md", who: "human", l: "exec",
  t: "The dependency licence inventory",
  m: O + `exec cmd="cargo deny list -f human -l license" inputs=Cargo.lock,deny.toml name=licences as=fence` + C,
  d: `Legal asks for it once a year, and it is regenerated by hand every time, from scratch, by whoever is free.`,
  c: `Prefer <code>cargo deny list</code>: <code>cargo license</code>'s column order is undocumented — its README is a screenshot.`,
  xk: "shape",
  x: `MIT OR Apache-2.0 (9)  anyhow, clap, globset, ignore, libc, serde, sha2, tempfile, toml
Apache-2.0 (1)         similar
MIT (1)                wait-timeout` },

{ g: "Human-first", n: 20, f: "README.md", who: "human", l: "exec",
  t: "The release platform matrix, from the release workflow",
  m: O + "exec cmd=scripts/platforms.sh inputs=.github/workflows/release.yml,scripts/platforms.sh name=platforms as=fence" + C,
  d: `Measured here: the README names four targets in prose and <code>release.yml</code> names them in a matrix. Two copies, one commit away from disagreeing. TinyGo has an open issue that is exactly this — “Go compatibility matrix is out of date”.`,
  xk: "real",
  x: `aarch64-apple-darwin        macos-14
x86_64-apple-darwin         macos-14
x86_64-unknown-linux-musl   ubuntu-latest
aarch64-unknown-linux-musl  ubuntu-24.04-arm` },

{ g: "Human-first", n: 21, f: "README.md", who: "human", l: "exec", shipped: true,
  t: "Install instructions carrying the current version",
  m: O + "exec cmd=scripts/install-block.sh inputs=Cargo.toml,scripts/install-block.sh name=install as=fence" + C,
  d: `Bun puts the version in the install button. Most projects' install snippets pin a version that shipped two releases ago. release-please solves the same problem with inline version annotations. This is the region running on this site's own landing page.`,
  xk: "live",
  x: `cargo install computed   # v0.1.0
computed run             # fill what moved
computed check           # exit 1 on drift` },

{ g: "Human-first", n: 22, f: "docs/todo.md", who: "human", l: "exec",
  t: "The TODO inventory, with age",
  m: O + "exec cmd=scripts/todo-age.sh inputs=src/*.rs,scripts/todo-age.sh name=todos as=fence" + C,
  d: `<code>git blame --porcelain</code> gives an author time per line; <code>tickgit</code> productises it, printing “added 1 month ago by …” and a total. A list nobody keeps because keeping it is the whole job.`,
  c: `<code>inputs=</code> cannot see <code>.git</code>, so the age is not what makes the region stale — the TODO lines are.`,
  xk: "shape",
  x: `src/render.rs:214  TODO drop the clone once Rendered borrows   added 8 months ago
src/loader.rs:377  FIXME the timeout kills the group, not the tree  added 3 months ago` },

{ g: "Human-first", n: 23, f: "README.md", who: "human", l: "exec",
  t: "Coverage, as one number",
  m: O + `exec cmd="cargo llvm-cov --json --summary-only | jq -r '.data[0].totals.lines.percent'" inputs=src/*.rs,tests/*.rs name=coverage` + C,
  d: `Churns on nearly every commit — which is exactly why the skip-if-fresh rule makes it viable here and unusable under a rerun-to-check tool. Extract the one field. Never paste the table.`,
  xk: "shape",
  x: `88.4` },

{ g: "Human-first", n: 24, f: "README.md", who: "human", l: "exec",
  t: "The release binary size",
  m: O + `exec cmd="du -h target/release/computed | cut -f1" inputs=src/*.rs,Cargo.lock name=binsize` + C,
  d: `A number every systems project claims in prose and nobody re-measures. Tie it to the sources and the lockfile and it re-measures itself.`,
  c: `<code>size(1)</code>'s column spacing is not guaranteed identical across platforms; <code>du</code> and <code>ls</code> are the portable options.`,
  xk: "real",
  x: `2.9M` },

{ g: "Human-first", n: 25, f: "docs/benchmarks.md", who: "human", l: "exec",
  t: "A benchmark table, with the command as a column",
  m: O + "exec cmd=scripts/bench.sh inputs=benches/*,scripts/bench.sh name=bench as=fence" + C,
  d: `The single highest-value block on any landing page, and nobody has a current one. esbuild's chart is hardcoded HTML; uv's is a PNG dragged into a comment box, in a repository whose own <code>BENCHMARKS.md</code> documents the command that generates the data; ripgrep's tables are refreshed by hand.`,
  xk: "shape",
  x: `benchmark                     median    runs
parse and hash 1 000 regions   38 ms     200
walk 10 000 paths              21 ms     200` },

{ g: "Human-first", n: 26, f: "docs/locales.md", who: "human", l: "exec",
  t: "Any list currently carrying a “may be out of date” disclaimer",
  m: O + "exec cmd=scripts/inventory.sh inputs=<the directory>,scripts/inventory.sh name=inventory as=fence" + C,
  d: `The disclaimer is the tell. Four real ones were found in the survey, and every one is attached to a list that could be enumerated from a directory. TypeScript's even links to the directory it should have walked.`,
  xk: "shape",
  x: `de-DE.json
en-US.json
fr-FR.json
ja-JP.json
pt-BR.json` },

{ g: "Serving both", n: 27, f: "README.md", who: "both", l: "exec",
  t: "The “these copies must stay in sync” ledger",
  m: O + `exec cmd="cat docs/tables/states.md" inputs=docs/tables/states.md name=states as=raw` + C,
  d: `Compute the canonical table once, embed the same region in every file that needs it, and let <code>check</code> catch a divergence. Measured here: the exit-code table lives in four files, the command synopsis in four, the state table in four — and exactly one of those copies is a region. Kubernetes hand-rolls this with seventy-six verify/update script pairs.`,
  xk: "shape",
  x: `| State | run | check |
|---|---|---|
| fresh | leaves it alone | passes |
| stale | re-renders | exit 1 |
| edited | refuses the file | exit 1 |
| volatile | re-renders every time | passes |` },

{ g: "Serving both", n: 28, f: "README.md", who: "both", l: "exec",
  t: "The config schema, derived from the parser",
  m: O + "exec cmd=scripts/grammar-table.sh inputs=src/marker.rs,scripts/grammar-table.sh name=grammar as=raw" + C,
  d: `The best single region this project could add. The loader and attribute table is a constant in <code>src/marker.rs</code>, and it is hand-copied into the README, the reference, this site and the spec. <code>schemars</code> does the equivalent for serde types.`,
  xk: "real",
  x: `| Loader | Attributes           | Flags     | Default sink |
|--------|----------------------|-----------|--------------|
| tree   | src, depth           | all, dirs | fence        |
| exec   | cmd, inputs, timeout | volatile  | raw          |

Common to both: name, as, lang` },

{ g: "Serving both", n: 29, f: "docs/spec/computed-v0.md", who: "both", l: "exec",
  t: "Spec coverage by tests — which section of the spec has a test",
  m: O + "exec cmd=scripts/spec-coverage.sh inputs=docs/spec/computed-v0.md,tests/*.rs,scripts/spec-coverage.sh name=speccov as=fence" + C,
  d: `CommonMark is the model: the prose spec is the fixture, and its extractor tags every case with the spec heading it sits under. Web platform tests require a link to the section under test but ship no first-party coverage map.`,
  xk: "shape",
  x: `3.1  Markers          tests/render.rs   6 cases
3.2  The two sums     tests/render.rs   4 cases
4    States           tests/render.rs  11 cases
5    Trust            tests/cli.rs      2 cases
6    Exit tiers       tests/cli.rs      4 cases` },

{ g: "Serving both", n: 30, f: "README.md", who: "both", l: "exec",
  t: "Every skill or command a plugin ships",
  m: O + "exec cmd=scripts/skills.sh inputs=claude-code-plugin/skills/*/SKILL.md,scripts/skills.sh name=skills as=fence" + C,
  d: `Name and description from each skill's own frontmatter. This repository's README lists its two skills by hand, and a third would not appear.`,
  xk: "real",
  x: `computed-setup    Install the computed CLI and wire it into a repository, the
                  pre-commit hook and the CI check.
discover-regions  Find hand-written blocks in a repository's Markdown that go
                  stale on their own, and turn them into computed regions.` },

{ g: "Serving both", n: 31, f: "CLAUDE.md", who: "both", l: "exec", shipped: true,
  t: "The index of decisions",
  m: O + "exec cmd=scripts/adr-index.sh inputs=docs/adr/*.md,scripts/adr-index.sh name=adrs" + C,
  d: `Already running in this repository's <code>CLAUDE.md</code>. <code>adr-log</code> is the prior art and uses the same marker shape, with no check mode; <code>adr generate toc</code> only prints to stdout and leaves a human to place it.`,
  xk: "live",
  x: `- [Rust for the prototype](docs/adr/0001-rust-for-the-prototype.md)
- [Two sums in the closer](docs/adr/0002-two-sum-closer.md)
- [In-place is the only layout in v0](docs/adr/0003-in-place-layout.md)
- [Relative paths resolve against the template's directory](docs/adr/0004-region-root-is-the-template-directory.md)
…` },

{ g: "Serving both", n: 32, f: "CLAUDE.md", who: "both", l: "tree", shipped: true,
  t: "The file tree",
  m: O + "tree src=. depth=2 name=layout" + C,
  d: `Already running here. Measured: the body changed in eight of the forty-one commits since this repository started using the tool. The boring baseline, and still the one most repositories get wrong.`,
  xk: "live",
  x: `.
├── CLAUDE.md
├── CONTEXT.md
├── Cargo.lock
├── Cargo.toml
├── LICENSE-MIT
├── README.md
├── claude-code-plugin
│   └── skills
├── docs
│   ├── adr
│   ├── catalogue.html
│   ├── how-it-works.html
│   ├── index.html
│   ├── research
│   └── spec
…` },

{ g: "Serving both", n: 33, f: "claude-code-plugin/skills/REFERENCE.md", who: "both", l: "exec", shipped: true,
  t: "A command's synopsis, from its own --help",
  m: O + "exec cmd=scripts/cli-commands.sh inputs=src/cli.rs,scripts/cli-commands.sh name=commands as=fence" + C,
  d: `Already running in this repository's agent reference. An open issue on adobe/aio-cli is the same problem, unsolved for three years. simonw/llm solves it with cog, at the cost of needing the interpreter, the package and a doctored environment in CI.`,
  xk: "live",
  x: `computed run      [paths] [--force] [--dry-run] [--trust]
computed check    [paths]
computed clean    [paths] [--force] [--dry-run]
computed trust    [path]
computed untrust  [path]` },

{ g: "Files that are not documentation", n: 34, f: "docs/spec/computed-v0.md", who: "both", l: "exec",
  t: "Inside a spec — its own crate layout and dependency list",
  m: O + "exec cmd=scripts/module-map.sh inputs=src/*.rs,scripts/module-map.sh name=modules as=fence" + C,
  d: `This project's spec hand-lists eight modules and eleven crates in a section called “How is the crate laid out?”. CommonMark's spec carries its own test cases inline; a spec carrying its own current inventory is the same move.`,
  xk: "real",
  x: `cli.rs      The five commands: clap definitions, discovery, per-file context…
fs.rs       The walk, the repository root, and the atomic write.
loader.rs   The two loaders, tree and exec, and the production Loaders adapter.
marker.rs   The marker grammar: parse a file into prose and regions, and serialise it back.
render.rs   Decides what every region becomes. Pure: a parsed file, a mode, a trust…
sink.rs     The two sinks, raw and fence, and the normalisation every loader's…` },

{ g: "Files that are not documentation", n: 35, f: ".github/ISSUE_TEMPLATE/bug.yml", who: "both", l: "exec",
  t: "Inside an issue template — the version dropdown",
  m: O + "exec cmd=scripts/versions-yml.sh inputs=Cargo.toml,scripts/versions-yml.sh name=versions as=raw" + C,
  d: `GitHub's own flagship <code>dropdown</code> example is a version list, and a dedicated Action exists solely to rewrite it — its dogfood fixture ships the template with empty placeholder options for the Action to fill. A region does it in the file, with a check.`,
  xk: "shape",
  x: `- type: dropdown
  id: version
  attributes:
    label: Version
    options:
      - 0.1.0
      - main (unreleased)` },

{ g: "Files that are not documentation", n: 36, f: ".github/pull_request_template.md", who: "human", l: "exec",
  t: "Inside a pull request template — the checks CI will run",
  m: O + "exec cmd=scripts/ci-steps.sh inputs=.github/workflows/ci.yml,scripts/ci-steps.sh name=ci as=fence" + C,
  d: `A pull request template is Markdown at a path git already knows. Nothing restricts its content to hand-written prose, and its checklist is the thing most likely to describe a pipeline from a year ago.`,
  xk: "real",
  x: `- [ ] cargo build --release
- [ ] cargo test
- [ ] cargo clippy --all-targets -- -D warnings
- [ ] cargo fmt --check
- [ ] computed check . docs/index.html` },

{ g: "Files that are not documentation", n: 37, f: "tests/render.rs", who: "agent", l: "exec",
  t: "Inside a test file — the fixture manifest beside the loop that reads it",
  m: O + "exec cmd=scripts/fixtures.sh inputs=tests/fixtures/*,scripts/fixtures.sh name=fixtures as=raw" + C,
  d: `Machine-owned regions inside source are already normal: <code>expect-test</code> rewrites them with <code>UPDATE_EXPECT=1</code>, <code>insta</code> stamps a generated header onto every snapshot, <code>sqllogictest-rs --override</code> rewrites expected rows in place.`,
  xk: "real",
  x: `fresh.check.report.txt
fresh.clean.txt
fresh.in.txt
fresh.stale-layout.txt
states.check.report.txt
states.dry-run.txt
states.force.txt
states.in.txt
…` },

{ g: "Files that are not documentation", n: 38, f: "CONTRIBUTING.md", who: "human", l: "exec",
  t: "Inside CONTRIBUTING.md — the verify and update command pair",
  m: O + "exec cmd=scripts/ci-steps.sh inputs=.github/workflows/ci.yml,scripts/ci-steps.sh name=ci as=fence" + C,
  d: `Kubernetes documents this by hand — run <code>hack/verify-all.sh</code> before a pull request, and <code>hack/update-all.sh</code> if it fails — and the list drifts from the workflow like everything else.`,
  xk: "real",
  x: `Before opening a pull request:

  cargo test
  cargo clippy --all-targets -- -D warnings
  cargo fmt --check
  computed check . docs/index.html` },

{ g: "Files that are not documentation", n: 39, f: "docs/index.html", who: "both", l: "exec", shipped: true,
  t: "Inside the project's own website — the tables it copies from the docs",
  m: O + "exec cmd=scripts/commands-html.sh inputs=src/cli.rs,scripts/commands-html.sh name=commands as=raw" + C,
  d: `Running on the page you came from: its install block is an <code>as=raw</code> region over <code>Cargo.toml</code>, and <code>computed check . docs/index.html</code> runs in this project's CI. <code>computed</code> reads a named file whatever its extension, so the only thing that had to change was the path list.`,
  c: `The <code>fence</code> sink does not port — three backticks in an HTML file are three backticks. Use <code>as=raw</code> and an HTML-shaped generator until a <code>pre</code> sink exists.`,
  xk: "live",
  x: `<pre class="mt-6 rounded border border-line bg-ground-2 p-4 …"><code>cargo install computed   <span class="text-ink-2"># v0.1.0</span>
computed run             <span class="text-ink-2"># fill what moved</span>
computed check           <span class="text-ink-2"># exit 1 on drift</span></code></pre>` },

{ g: "Files that are not documentation", n: 40, f: "CHANGELOG.md", who: "human", l: "exec",
  t: "Inside CHANGELOG.md — the unreleased section",
  m: O + `exec cmd="git cliff --unreleased" volatile name=unreleased as=raw` + C,
  d: `git-cliff generates from conventional commits and has no check mode; verification is generate-and-diff.`,
  c: `Git history is not expressible as <code>inputs=</code>, so this must be <code>volatile</code>, which means <code>check</code> always passes it.`,
  xk: "shape",
  x: `## [Unreleased]

### Added
- a pre sink, so an HTML file can carry a fenced region

### Fixed
- inputs= no longer matches a directory` },
];

window.COMPUTED_REGIONS = ENTRIES;
})();
