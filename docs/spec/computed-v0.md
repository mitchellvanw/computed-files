# computed v0

`computed` keeps marked regions of a markdown file current by computation. The document is a view; the truth lives in a directory listing or the output of a command. This is the specification for v0: the marker grammar, the two loaders, the freshness model, the hand-edit policy, the trust model, the command line, the crate layout, and the definition of milestone 1. Every decision here was made on a ticket and, where it is hard to reverse, recorded as an ADR under [`docs/adr/`](../adr/). The spec gists; the ADR argues. Vocabulary is [`CONTEXT.md`](../../CONTEXT.md), and this document uses its terms without redefining them.

The first reader is an `/implement` session building milestone 1 against this repository's own `CLAUDE.md`. Anything the spec leaves open is listed at the end, with the reason it is open.

## What is v0 for?

Two users: a developer on macOS and a CI job. One dogfood target: this repository. The foundation is `run` in a pre-commit hook and `check` in CI. A watcher is a convenience layer and is not in v0 ([Is `watch` in v0](#is-there-a-watch-command)).

The problem v0 solves, stated once: every surveyed tool that rewrites a marked region remembers what it wrote, not what it read. To answer "is this file fresh" they run every generator again, which is the slow path and, for shell regions, the untrusted one. `computed` stores both an input sum and an output sum in the file, so `check` answers from the file and its inputs alone ([ADR 0002](../adr/0002-two-sum-closer.md), [ADR 0006](../adr/0006-check-never-runs-a-loader.md)).

Rust, one static binary, no runtime to install ([ADR 0001](../adr/0001-rust-for-the-prototype.md)).

## What does a region look like?

A region is the span between an opener and a closer. Both are whole-line HTML comments. Rendered, a region looks like this:

````markdown
<!-- computed tree src=. depth=2 name=layout | do not edit; run computed -->
```text
.
├── docs
└── src
```
<!-- /computed in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715 -->
````

The prose around the markers is the author's. The body between them belongs to the tool.

### Markers

A marker is a whole line: optional leading whitespace, which is preserved, the comment, optional trailing whitespace, which is ignored. No other container is a marker: not a link, not inline code, not a fence. Markers inside CommonMark fenced code blocks (backtick or tilde, any length, with a matching closer) are ignored, so a document can show marker examples. Indented code blocks are not tracked in v0.

The opener:

```
<!-- computed <loader> [flag ...] [key=value ...] [| do not edit; run computed] -->
```

- `computed` is lowercase and case-sensitive. Tokens are separated by any run of spaces or tabs. The tool writes canonical single-space form.
- The first bare word is the loader. Later bare words are boolean flags.
- Attributes are `key=value`. A value is unquoted, or double-quoted when it contains whitespace or `>`. The only escape is `\"`. A value may not contain `-->`.
- `name=` is optional and unique per file. Reports use the name, else `loader@line`.
- `as=` selects the sink. Per-loader default: `tree` gets `fence`, `exec` gets `raw`. `lang=` sets the fence language, default empty.
- The suffix `| do not edit; run computed` is written by the tool into the rendered opener and stripped by the parser. A template does not need it.

The closer:

```
<!-- /computed [in=<hex> out=<hex>] -->
```

`in` is the input sum, `out` the output sum. Both present or neither. A closer with neither means the region is unrendered. A closer with one is a parse error.

The body is the lines strictly between the marker lines. The tool writes what the sink produced and adds no blank lines of its own; the sink emits the blank lines CommonMark needs.

### Parse errors

All hard, the file is not written, exit 2: unknown loader, unknown attribute, duplicate name, opener without closer, closer without opener, opener inside a body (nesting is not supported), one-sum closer, malformed sum, value containing `-->`.

### Why two sums?

[ADR 0002](../adr/0002-two-sum-closer.md). One sum over the output, cog's model, detects hand edits but cannot tell whether the inputs moved. Sums in a sidecar are a second file to lose. Two sums in the closer let a fresh clone carry everything `check` needs.

## Which layout?

In-place only. The template and the rendered file are one file. There is no file-level banner; the opener suffix is the generated signal, true at the granularity it claims ([ADR 0003](../adr/0003-in-place-layout.md)).

Writes go through a temp file in the same directory and `rename(2)`, with the original's mode bits copied. When the rendered bytes equal the current bytes nothing is written and the mtime is untouched, so `run` twice is a no-op.

Copy layout, a `.tmpl` rendered to the canonical path, was the research recommendation and lost: it reverts every prose edit made to the rendered file, which on an agent-edited `CLAUDE.md` is the normal case. The parser, render and sum core stay layout-agnostic so copy can return as an opt-in.

## Which loaders?

Two: `tree` and `exec`. Modelled as `enum Loader { Tree(TreeArgs), Exec(ExecArgs) }`, a closed set, so an enum and not a trait. Both produce the same thing, `Loaded { text: String, snapshot: Vec<u8> }`: native loaders and commands are one kind of thing at the data level.

Common attributes: `name=`, `as=`, `lang=`. Each loader owns its own attribute set. Any other attribute is the grammar's unknown-attribute error.

### Paths and the region root

Every relative path in a marker resolves against the template's directory, the region root, and an exec command runs with that directory as its working directory ([ADR 0004](../adr/0004-region-root-is-the-template-directory.md)). Neither the repository root nor the invocation directory: the first makes a nested file spell its own path back, the second makes the same file render differently from a hook, from CI and from a shell.

`tree src=` and exec `inputs=` must resolve inside the repository root, or inside the region root when the file is not in a repository. A path that escapes is a hard error. This is a reproducibility rule, not a security fence: a region reading outside the repository renders differently on every machine.

### `tree`

- `src=` default `.`. `depth=` default unlimited, counted as `tree -L n` counts. Flags: `all` includes dotfiles, `dirs` lists directories only.
- Output in `tree`'s box-drawing style with a `.` root line. No sizes, mtimes or counts.
- Gitignore-aware through the `ignore` crate, configured as the research settled ([ignore semantics](../research/ignore-gitignore-semantics.md)): `.hidden(true)`, `.ignore(false)`, `.git_ignore(true)`, `.parents(true)`, `.require_git(true)`, `.git_exclude(false)`, `.git_global(false)`, `.follow_links(false)`, byte-order sort by file name, the sequential walker. Per-clone and per-user exclude files are off because they would make the snapshot differ between machines. `tree --gitignore` is not git-exact and is not a byte-for-byte reference. The rules apply inside a repository without any flag, and not at all outside one; `gitignore` is not an attribute ([ADR 0011](../adr/0011-gitignore-is-not-a-flag.md)).
- One walk, byte order of names, directories and files interleaved. The rendered listing and the snapshot are the same sequence. A directory whose children are all ignored is still listed.
- Snapshot: one relative path per line, LF-terminated, directories with a trailing `/`, limited by `depth`, `all` and `dirs` exactly as the listing is.
- Default sink `fence`.

### `exec`

- `cmd=` required. Run as `/bin/sh -c "<cmd>"`, never the login shell. stdin is closed.
- Exactly one of `inputs=` or the `volatile` flag. Neither, or both, is a parse error.
- `inputs=` is comma-separated globset syntax: `**`, `*`, `?`, `[..]`, no braces. Ignore rules do not filter declared inputs. A glob that matches nothing is a hard error under `run` and under `check`. A matched directory means every file under it. The template file itself is silently excluded from its own snapshot.
- Snapshot with `inputs=`: for each matched file in byte-order sorted relative path, `path` `\0` decimal byte length `\0` content `\0`. `volatile`: an empty snapshot.
- `timeout=` in seconds, default 30. Expiry kills the process group and counts as failure.
- Environment: the inherited environment with `LC_ALL=C`, `LANGUAGE=` (empty) and `TZ=UTC` set unconditionally, plus `COMPUTED_FILE` (the template path), `COMPUTED_ROOT` (the repository root, unset outside one) and `COMPUTED_REGION` (name or `loader@line`). Nothing else is touched, `PATH` included. A command that wants a locale or zone sets it inside `cmd=` ([ADR 0009](../adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md)).
- stdout must be UTF-8; otherwise the loader failed.
- Default sink `raw`.

### Loader failure

Non-zero exit, timeout, invalid UTF-8, or text that fails normalisation (next section): the previous body is kept, the sums are kept, the command's stderr is reported under the region's line, nothing is written into the region, and the run exits 1. Deleting an input does not blank a region; restoring it and running again repairs it.

### Which loader is not here?

`file`, a verbatim include that needs no trust, is wanted only if an untrusted repository turns out to need includes. This repository is trusted, so dogfooding cannot show that. It stays in the fog.

## What makes loader text deterministic?

The output sum is taken over body bytes, so anything that changes those bytes for the same inputs is drift. Two sources of change are not inputs at all: the shape of the text a command prints, and the environment it prints under. Both are fixed before a sink sees the text ([ADR 0009](../adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md)).

Normalisation, applied in the `sink` module to every loader's text before either sink shapes it, in order:

1. Invalid UTF-8, any C0 control byte other than tab, LF and CR, or a line that would parse as a marker opener or closer: loader failure.
2. CRLF and lone CR become LF.
3. Trailing newlines are stripped. Trailing spaces and tabs on a line are kept: markdown gives them meaning and padded tables carry them. Empty output stays empty.

The sink then owns the line structure. `fence` writes the opening fence with `lang=`, the text, the closing fence. `raw` writes a blank line, the text, a blank line. Every line is LF-terminated. Interior blank lines are untouched.

One rule was added while assembling this spec, because the grammar skips markers inside fences and loader text can contain fence lines: after the sink has shaped the body, the region must parse back to the same body. `fence` guarantees this by choosing a backtick run one longer than the longest backtick run that starts a line of the text, minimum three. For `raw`, text whose fences are unbalanced would swallow the closer on the next parse; that is a loader failure, reported like rule 1. This keeps the invariant that a file the tool wrote always parses.

A change to any normalisation rule bumps both loaders' format constants.

## When is a region fresh?

A region carries two sums in its closer. Both are SHA-256, stored as the full 64 lowercase hex characters ([ADR 0010](../adr/0010-sha-256-sums.md)).

**Input sum.** SHA-256 over, in order: the domain line `computed-in/1\n`; the loader and its format constant, `<loader>/<n>\n`; the canonical opener line (single-space tokens, suffix stripped, indentation stripped) followed by `\n`; the snapshot bytes.

**Output sum.** SHA-256 over the body bytes exactly as they sit between the marker lines, each line with its terminator. An empty body hashes empty.

**Format constant.** Each loader carries an integer, starting at 1, bumped by hand only when that loader's output for the same inputs changes. It is hashed and never printed. The crate version is not in the sum: upgrading the binary re-renders only the loaders whose rendering changed, and a newer local binary cannot produce drift that CI cannot reproduce.

**States.** Derived from the file and its inputs alone:

| State | Meaning |
|---|---|
| `fresh` | Recomputed input sum equals `in=`, body sum equals `out=`. The only state `run` leaves untouched. |
| `stale` | Input sum differs. Inputs or the opener changed. |
| `edited` | Body sum differs. Someone changed the body by hand. |
| `stale+edited` | Both. |
| `volatile` | Declares no snapshot and the body matches `out=`. |
| `unrendered` | The closer carries no sums. Rendered without regard to the body. |

A volatile region whose body does not match `out=` is `edited`. Volatile exempts only the input side of the test, so a hand edit inside a volatile body is still caught.

**The cache.** `run` skips the loader of a fresh region. A loader therefore has two steps: `snapshot`, computed before any work and always run by both `run` and `check`, and `load`, run only when the region is stale, unrendered, or volatile. For `tree` both steps are one walk. Fresh regions are reproduced byte-for-byte from their raw lines: canonical spacing and the opener suffix appear the first time a region renders, so a clean `check` guarantees `run` is a no-op.

**`check` never runs a loader** ([ADR 0006](../adr/0006-check-never-runs-a-loader.md)). It computes snapshots, compares both sums and reports states. So `check` is safe on an unvetted clone and cheap in a hook, and it cannot show the diff `run` would write. That diff lives on `run --dry-run`. The cost, accepted: a loader whose output changes without an input or format-constant change is invisible until `--force`.

## What happens to a hand edit?

`run` refuses ([ADR 0005](../adr/0005-refuse-hand-edited-regions.md)). A region is edited when its body does not match `out=`. Only the body is subject to this test.

- **Default.** The file is not written, the region is named (file, line, `name=` when present), and the invocation exits 1. There is no policy switch in v0.
- **`--force`.** `run --force` overwrites every edited region in every file the invocation processes. Narrow the scope by passing paths. `check --force` is a usage error.
- **Per file.** A file with one edited region is left untouched in full, merely stale regions included. Other files in the same invocation are rendered and written. The exit code is 1 when any file was refused.
- **Openers and closers never reach this policy.** An edited opener changes the input sum: the region is `stale` and re-renders. A closer missing both sums is `unrendered` and renders whatever the body contains. Any other closer damage is a parse error and nothing in the file is written.

Prose outside a region needs no policy. After a prose edit `run` renders the identical file and skips the write.

Why refuse, when every tool but cog overwrites? The hand-edit prototype ([`prototypes/hand-edit.prototype.html`](../../prototypes/hand-edit.prototype.html)) showed the one case that matters: a body edit and an input change landing together. Under overwrite the edit vanishes inside a legitimate re-render, indistinguishable from a normal run. Under warn the hook writes and exits 0, so on the pre-commit path it is overwrite with a line nobody reads. On `CLAUDE.md` the editor is usually an agent that does not know the region is owned, and a failed hook is the one signal that reaches it.

## Who may run a command?

An exec region runs only when the repository it sits in has been trusted on this machine ([ADR 0007](../adr/0007-exec-trust-per-clone.md)). The model is direnv's and git's `safe.directory`.

**What it defends against.** One thing: cloning a repository and having `computed run` execute its commands before anyone read them. A malicious branch pulled into a trusted clone, or a dependency writing markers into a file, runs on the next `run` exactly as a Makefile or a pre-commit hook would. We state this as accepted rather than imply a guarantee the model cannot keep.

**Grant.** `computed trust [path]` records a grant for the repository root containing `path` (default: the current directory), found by walking up for `.git`; outside a repository, the directory itself. It prints the root it recorded. `computed untrust [path]` removes it. The store is `$XDG_CONFIG_HOME/computed/trust.toml`, default `~/.config/computed/trust.toml`, on every platform, one entry per canonical root with symlinks resolved. Nothing inside the working tree can grant trust.

**One shot.** `run --trust` and `run --dry-run --trust` treat every file in the invocation as trusted without writing the store. This is how CI expresses trust. There is no environment variable. A `check`-only pipeline needs no trust at all.

**Lookup.** Per template file, against that file's own repository root (the same value as `COMPUTED_ROOT`), or its region root outside a repository. A submodule is a separate repository with its own grant.

**Untrusted.** `run` skips every exec region: body kept, nothing written into it, the region reported as `untrusted` with file, line and name, and the run exits 1. Tree regions in the same file still render and the file is still written. `check` never runs a loader, so trust never enters it.

## What is the command line?

Five commands. `--help` and `--version` come from clap. `-v` is global and shows the regions that are otherwise silent.

```
computed run   [paths] [--force] [--dry-run] [--trust]
computed check [paths]
computed clean [paths] [--force] [--dry-run]
computed trust   [path]
computed untrust [path]
```

A flag a command does not take is a usage error: `check --force`, `check --trust`, `clean --trust`. No `-C`, no stdin, no `-`.

**Discovery.** With no paths, walk the current directory with the same `ignore` settings as the tree loader and read only `.md` files. An explicit file is read whatever its extension. An explicit directory is walked. A file with no opener is skipped silently. A path that does not exist is a usage error. Files are processed in byte-order sorted path order.

**Exit codes.**

| Exit | Meaning | Examples |
|---|---|---|
| 0 | Nothing to report. | Everything fresh. |
| 1 | The content said no. | Drift under `check`; a refused file, a loader failure or an untrusted region under `run`; a file `--dry-run` would have changed. |
| 2 | The tool could not answer. | Usage error, marker parse error, path escaping the root, `inputs=` matching nothing, unreadable file. |

The invocation exits with the highest tier any file hit. A tier-2 file is skipped whole; other files are still processed and written.

**`run --dry-run`.** Renders everything `run` would, prints a unified diff per file that would change to stdout, writes nothing. Refused files print their refusal line and no diff. Exit code as `run`, plus 1 when any file would have changed. This is the only path that shows the pending diff.

**Reporting.** One line per region on stderr, columns `path:line`, name (blank when absent), loader, state, action:

```
CLAUDE.md:12 layout tree stale      written
CLAUDE.md:40 deps   exec edited     refused; run with --force
CLAUDE.md:58        exec untrusted  skipped; run `computed trust`
```

Fresh regions, and volatile regions under `check`, print only with `-v`. Nothing is printed and the exit is 0 when everything is fresh. Loader stderr is printed indented under the region's line. `check` uses the same shape without the action column. Diffs are the only thing on stdout. No colour and no machine-readable output in v0.

**`clean`.** Empties every region body and strips both sums from the closer, leaving the region unrendered with its opener line unchanged. Markers stay, so the next `run` rebuilds the region. Runs no loader and needs no trust. Honours the hand-edit policy: an edited region refuses the file, `--force` overrides. Takes paths and `--dry-run` like `run`.

### Hooks

Pre-commit runs `run`. CI runs `check`. The committed configuration for the pre-commit framework:

```yaml
repos:
  - repo: local
    hooks:
      - id: computed
        name: computed run
        entry: computed run
        language: system
        always_run: true
        pass_filenames: false
```

`always_run` and `pass_filenames: false` are required. A staged change under `src/` makes a tree region in `CLAUDE.md` stale without staging `CLAUDE.md`, and a filename-filtered hook would never see it. When the hook rewrites a file the commit fails and the user re-stages, as with rustfmt. On a fresh clone the hook fails with the `untrusted` line, which is the trust model working.

Without the framework, `.git/hooks/pre-commit` is two lines:

```sh
#!/bin/sh
exec computed run
```

### Is there a `watch` command?

`watch` is deferred. v0 is `run` in pre-commit and `check` in CI; a watcher is a convenience layer over `run` and is revisited only when dogfooding shows agent sessions reading stale regions between commits. Milestone 1 preserves what a later watcher relies on: temp-plus-rename writes that are a no-op when content is unchanged, the `out=` sum as own-write detection, one shared walk so watch scope equals discovery scope, and per-file refusal so `run` is re-entrant per file.

## How is the crate laid out?

Milestone 1 replaces `computed-proto` in place with one package, `computed`, edition 2021. `src/lib.rs` holds every module; `src/main.rs` calls `computed::cli::main()`. The prototype modules are deleted; what survives of them is already recorded as decisions ([ADR 0008](../adr/0008-render-is-pure-behind-a-loaders-seam.md)).

Dependencies: `clap`, `ignore`, `globset`, `sha2`, `tempfile`, `anyhow` (cli only), `similar`, `toml`, `serde`, `wait-timeout`, `libc`. No `regex`: the opener tokeniser is hand-written because of the quoting rules. Dev: `assert_cmd`, `tempfile`. No snapshot-testing crate.

Eight modules:

- **`marker`.** Parses a file into `File { segments }`, each `Segment::Prose(String)` or `Segment::Region(Region)`. `Region` carries `line`, `indent`, `raw_opener`, `raw_closer`, `body` as the bytes sit, the closer's `in`/`out` sums, and the parsed `Opener { loader, flags, attrs, name, sink, lang }`. `Opener::canonical()` gives the single-space form without suffix or indent. `serialise(&File) -> String` is the inverse. Parse errors are `ParseError { line, message }`, all tier 2.
- **`loader`.** `enum Loader { Tree(TreeArgs), Exec(ExecArgs) }` built from an `Opener`; `format_constant() -> u32`; the production `Loaders` adapter. Resolves `src=` and `inputs=` against a per-file `Ctx { template, region_root, repo_root: Option<PathBuf> }` and makes the escape check there, the only place marker paths are resolved. Derives the three `COMPUTED_*` variables. `LoadError::Hard(String)` is tier 2 (escape, empty glob, unreadable input); `LoadError::Failed { stderr }` is tier 1 (exit status, timeout, bad UTF-8, failed normalisation).
- **`sink`.** `raw` and `fence`, pure: `Loaded` text in, body out. Normalisation and the parse-back check sit beside them.
- **`render`.** `file(parsed: &File, mode: Mode, trusted: bool, loaders: &mut dyn Loaders) -> Rendered`. Pure, no I/O. `Mode { Run { force }, DryRun { force }, Check, Clean { force } }`. `Loaders` has two methods: `snapshot(&Region) -> Result<Option<Vec<u8>>, LoadError>`, where `None` is volatile, and `load(&Region) -> Result<Loaded, LoadError>`. `Rendered { Written { text, regions }, Unchanged { regions }, Refused { regions }, Error { line, message } }`. `render` owns the sums as a private `render::sum`, the freshness cache, the states, the refuse rule and its per-file consequence, the untrusted skip, loader failure keeping the body, and `clean`. Fresh regions are emitted from their raw lines. Per region it returns `RegionReport { line, name: Option<String>, loader, state, action, stderr: Option<String> }`.
- **`fs`.** `walk(root, WalkOpts { depth, all, dirs }) -> impl Iterator<Item = Entry>` with the `ignore` settings in exactly one place, used by discovery and by `tree`; `repo_root(path) -> Option<PathBuf>`, walking up for `.git` and canonicalising; `write(path, text)` through temp and rename, no-op when unchanged.
- **`trust`.** The `trust.toml` store under `XDG_CONFIG_HOME`, store path injectable; `grant`, `revoke`, `is_trusted(root)`. Uses `fs::repo_root`.
- **`report`.** The stderr line per region, loader stderr indented beneath; the unified diff on stdout from old and new text via `similar`, only for `--dry-run`.
- **`cli`.** clap definitions, discovery, per-file `Ctx` and trust resolution, the mapping from `Rendered` to a write and an exit tier. The only module using `anyhow`.

Why a `Loaders` trait when the loader set is an enum? They are different things. The loader set is closed, so it is an enum until a third variant earns a trait. `Loaders` is a seam: it has two adapters from the first commit, the production enum and a table-driven fake, which is the bar for a seam being real.

### Tests

- `render` through a fake `Loaders` against golden files under `tests/fixtures`: template in, text and reports out, covering the state table, refuse ordering, untrusted, loader failure and every mode. The golden files also fix the sum vectors.
- `marker`: round trips on an untouched file and every grammar error.
- `sink`: per variant, plus a table of byte sequences for normalisation.
- `fs::walk`: a tempdir with a `.gitignore`.
- `trust`: an injected store path.
- `cli`: `assert_cmd` for the three exit tiers and the pre-commit scenario.

## What is milestone 1?

Milestone 1 is done when this repository dogfoods itself: a `CLAUDE.md` at the root with a tree region and an exec region, kept current by `computed run` in pre-commit and verified by `computed check` in CI. One `/implement` session, test-first at the seams above, committed to the current branch and reviewed with `/code-review`.

**Build**, in this order, each step green before the next:

1. `marker`: parse, canonical opener, serialise, round trip, every error.
2. `sink`: normalisation, `raw`, `fence`, the parse-back check.
3. `render` with the fake `Loaders`: the state table, the cache, refuse, untrusted, failure, `clean`, all four modes, golden fixtures.
4. `fs`: `walk`, `repo_root`, `write`.
5. `loader`: `tree` over `fs::walk`, `exec` over `/bin/sh` with the pinned environment, timeout and process-group kill, the escape check, the production `Loaders`.
6. `trust`: the store, `grant`, `revoke`, `is_trusted`.
7. `report` and `cli`: the five commands, discovery, exit tiers, `--dry-run` diffs.
8. Delete the prototype modules and the `demo` command. Rename the package to `computed`.

**Dogfood.** Create `CLAUDE.md` in this repository with two regions and let `computed run` fill them:

````markdown
## Layout

<!-- computed tree src=. depth=2 name=layout -->
<!-- /computed -->

## Decisions

<!-- computed exec cmd="grep -h '^# ' docs/adr/*.md" inputs=docs/adr/*.md name=adrs -->
<!-- /computed -->
````

The prose around them is whatever the repository wants agents to read. The tree region exercises the walk and the listing snapshot. The exec region exercises `inputs=`, the content snapshot, and trust. Run `computed trust` once on the developer machine.

**Ship with it.** `.pre-commit-config.yaml` as above, and a CI workflow that builds the binary and runs `computed check`.

**Acceptance.** All of these, run from the repository root:

- `computed run` on the fresh `CLAUDE.md` writes both regions and exits 1; a second `computed run` writes nothing and exits 0; `computed check` exits 0.
- Add a file under `src/` and `computed check` exits 1 with the `layout` region `stale`; `computed run` writes it.
- Edit a line inside the `adrs` body and `computed run` exits 1 with `refused`, leaving the file untouched; `computed run --force` overwrites; `computed check` exits 0.
- `computed untrust`, then `computed run` reports the `adrs` region `untrusted` and exits 1, still renders the tree region; `computed run --trust` renders both.
- `computed clean` then `computed check` exits 1 with both regions `unrendered`; `computed run` restores them and `computed check` exits 0.
- Remove the `out=` from one closer and `computed run` exits 2 with a parse error, writing nothing.
- `computed run --dry-run` after touching an input prints a unified diff to stdout, writes nothing, exits 1.
- `cargo test` passes and the `render` golden files contain the sum vectors.

**Not in milestone 1**, by decision: `watch` (milestone 2 or later, not scheduled), copy layout, a `file` loader, sinks beyond `raw` and `fence`, configuration files, colour or machine-readable reporting, Windows.

## What is still open?

Nothing here blocks milestone 1. Each item is in scope for v0 and waits on something dogfooding will show.

- **Sinks beyond `raw` and `fence`.** Whether a native table sink is needed once duckdb and sqlite3 produce markdown tables through `exec`, and whether sinks emit prettier range-ignore comments.
- **A `file` loader** as a trust-free include tier. Wanted only if an untrusted repository needs includes.
- **Configuration.** A `computed.toml`, or conventions only. Trust is settled outside the repository, so this hangs on file discovery alone.
- **Reporting.** Colour, machine-readable output for CI, and whether a refused region prints a diff of its edited body.
- **A second dogfood repository**, if one is named.

Out of scope for v0, and returning only if the destination is redrawn: copy layout, symlink layouts, file formats other than markdown, image sinks, nesting, regions that read other regions, Windows, Elixir.

## Decision record

| ADR | Decision |
|---|---|
| [0001](../adr/0001-rust-for-the-prototype.md) | Rust, one static binary. |
| [0002](../adr/0002-two-sum-closer.md) | Two sums in the closer. |
| [0003](../adr/0003-in-place-layout.md) | In-place is the only layout. |
| [0004](../adr/0004-region-root-is-the-template-directory.md) | Relative paths resolve against the template's directory. |
| [0005](../adr/0005-refuse-hand-edited-regions.md) | `run` refuses a hand-edited region. |
| [0006](../adr/0006-check-never-runs-a-loader.md) | `check` compares sums and never runs a loader. |
| [0007](../adr/0007-exec-trust-per-clone.md) | Exec trust is granted per clone, outside the working tree. |
| [0008](../adr/0008-render-is-pure-behind-a-loaders-seam.md) | Render is pure behind a `Loaders` seam. |
| [0009](../adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md) | Loader text is normalised and exec runs in a pinned environment. |
| [0010](../adr/0010-sha-256-sums.md) | Sums are full SHA-256. |
