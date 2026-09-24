# computed

Keep marked regions of a hand-written Markdown or code file current. The document is a view. The truth lives somewhere else: a directory listing, a part of another file, the repository's history, or the output of a command.

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
<!-- /computed in=4b0267a37bd1187b38918aa56d344242263571578d5b300fb415b2fb4260d898 out=45ca178e8d4046c3d6663949c214424beef2e9912da4931b45621e1e5ecbf559 -->
~~~

The prose around the markers is yours. The body between them belongs to the tool. `computed run` renders every region that needs it; `computed check` in CI exits 1 if anything drifted, without running a single command.

One static binary, Rust, no runtime to install. The full design is in [`docs/spec/computed-v0.md`](docs/spec/computed-v0.md); the decisions that are hard to reverse are under [`docs/adr/`](docs/adr/). This repository is its own first user: [`CLAUDE.md`](CLAUDE.md) carries a file tree and an index of the ADRs, kept current by `computed run` in pre-commit and verified by `computed check` in CI.

## Install

```
cargo install computed
```

Or take a prebuilt binary from the [latest release](https://github.com/mitchellvanw/computed-files/releases/latest) — macOS on Apple silicon or Intel, Linux on x86-64 or arm64, no toolchain needed. Each archive's SHA-256 is listed in the release's `SHA256SUMS`.

## For an agent

This repository is also a Claude Code plugin. Installing it hands an agent the marker grammar, the setup procedure and the region states in every project, without a checkout:

```
/plugin marketplace add mitchellvanw/computed-files
/plugin install computed@computed
/reload-plugins
/computed-setup
```

`/reload-plugins` makes the skills live in this session. `/computed-setup` installs and wires the tool, then offers `/discover-regions`, which finds the hand-written blocks worth computing and writes the markers.

The plugin also guards the agent's edits. Before an edit lands, a hook refuses one that changes a region's body and names the source to edit instead; after it, a second hook reports any region the edit left stale. `run` refusing a hand edit at commit time is the backstop, not the first signal. [ADR 0025](docs/adr/0025-the-guard-refuses-an-edit-before-it-lands.md); without the plugin, see [`docs/integrations.md`](docs/integrations.md#claude-code).

The two wizards are [`computed-setup`](claude-code-plugin/skills/computed-setup/SKILL.md) and [`discover-regions`](claude-code-plugin/skills/discover-regions/SKILL.md). They share [`REFERENCE.md`](claude-code-plugin/skills/REFERENCE.md), whose command block is itself a `computed` region over `src/cli.rs`, so CI fails if the command line moves and the skills do not.

## Why another region rewriter

Tools like cog, mdsh, and markdown-magic already rewrite the span between two comment markers. They share one gap: they remember what they wrote, not what they read. cog stores a checksum over the output, so it can tell you a region was hand-edited. It cannot tell you the inputs moved. To answer "is this file fresh" they have to run every generator again, which is the slow path and, for shell regions, the untrusted one.

The dogfood target is `CLAUDE.md`. Agents and people edit that file constantly. It sits at the root of a repo whose layout changes every day. A file tree pasted into it is wrong within a week, and nobody notices, because nothing checks.

## The two-sum closer

Every closing marker carries two sums, the full 64 hex characters of a SHA-256 hash each:

```
<!-- /computed in=4b0267a37bd1187b38918aa56d344242263571578d5b300fb415b2fb4260d898 out=45ca178e8d4046c3d6663949c214424beef2e9912da4931b45621e1e5ecbf559 -->
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

**Hand edits are refused, not silently reverted.** A file with one edited region is left untouched in full and the run exits 1 with the region named. `run --dry-run` shows what `--force` would do to it, and `run --force` overwrites; `--force` also re-renders fresh regions, for output that moved without its inputs. The reason is the case where a body edit and an input change land together: under overwrite the edit vanishes inside a legitimate re-render, and on `CLAUDE.md` the editor is usually an agent that does not know the region is owned. A failed hook is the one signal that reaches it. [ADR 0005](docs/adr/0005-refuse-hand-edited-regions.md).

**A loader failure keeps the last good body.** Non-zero exit, timeout, or output that fails normalisation: the body and sums stay as they were, the command's stderr is printed under the region's line, and the run exits 1 so CI notices. Restoring the input and running again repairs it. A region the tool cannot answer at all, such as `inputs=` matching nothing, is reported `error` and exits 2, and the other regions in its file are still kept current. [ADR 0013](docs/adr/0013-a-region-the-tool-cannot-answer-skips-only-itself.md).

**Templates that read each other settle in one run.** When one region's `inputs=` include another template, `run` renders again whatever read a file it just wrote, until nothing changes, and the sums in a closer are left out of any snapshot that reads them. [ADR 0014](docs/adr/0014-snapshots-ignore-sums-and-run-settles-across-files.md).

**`check` never runs a command from the repository.** It recomputes snapshots, compares both sums and reports. Snapshots read files, and for a `git` region they read history with `git`; they never run an exec command and never fetch. So `check` is safe on an unvetted clone and cheap in a hook. The diff `run` would write lives on `run --dry-run`. [ADR 0006](docs/adr/0006-check-never-runs-a-loader.md), [ADR 0018](docs/adr/0018-the-git-snapshot-runs-git-under-check.md).

## Loaders and sinks

A loader produces text and a snapshot of what it read. A sink shapes that text into what goes in the file.

| Loader | Renders | Trust |
|---|---|---|
| `tree` | a directory listing, `tree`-style, gitignore-aware | no |
| `file` | another file, or one slice of it: `lines=`, `section=`, `anchor=` | no |
| `value` | one field of a TOML, JSON or YAML file: `key=package.version` | no |
| `index` | a linked list of files matched by globs, titled by their first heading | no |
| `toc` | the file's own headings, with GitHub's anchors | no |
| `symbol` | one item of a Rust, Python or Go file: whole, its signature, or its doc | no |
| `git` | recent commits, tags or contributors | no |
| `remote` | a document fetched over HTTPS, pinned by its SHA-256 | no; an allowlist |
| `exec` | a command's output, with `inputs=` or `volatile` | yes |
| `transcript` | a shell session, each step and what it printed | yes |

Sinks are `raw` (Markdown as it stands), `fence` (a code block), `table`, which turns CSV, TSV or JSON Lines into a Markdown table, and `comment`, which writes each line behind a code file's comment leader. Every region also takes `name=`, `as=`, `lang=`, `max-lines=N`, which cuts long output to N lines and a note, and `on-stale=warn`, which lets `check` report the region stale without failing. The full grammar of each loader is in the [spec](docs/spec/computed-v0.md#which-loaders).

**A region reads only what it names.** The snapshot of `file src=README.md section=Install` is that section, and of `value src=Cargo.toml key=package.version` the version, so an edit anywhere else in the file leaves the region fresh. An `exec` input can be narrowed the same way: `inputs="Cargo.toml#key=package.version,src/*.rs"`. [ADR 0016](docs/adr/0016-a-projection-snapshots-only-the-part-it-reads.md).

**Native loaders need no trust.** Every loader but `exec` and `transcript` reads files, history or a pinned document and runs nothing from the repository, so a fresh clone renders them without a grant. Before, an ADR index or a version string was a script, and a fresh clone's hook failed on it until someone ran `computed trust`. [ADR 0017](docs/adr/0017-trust-gates-running-repository-code-and-nothing-else.md).

**A long opener can be named once.** A `computed.toml` holds recipes, `[recipe.adrs]` with a loader and its attributes, and a region writes `use recipe=adrs`. The recipe's expansion is the opener for every purpose, sums included. [ADR 0022](docs/adr/0022-recipes-in-computed-toml.md).

**A region can hold one value in a sentence.** In Markdown and HTML, an opener and a closer on the same line make an inline region: `The current release is <!-- computed value src=Cargo.toml key=package.version -->0.2.0<!-- /computed in=… out=… -->.` Its text must be one line. Markers inside backticks stay prose, so a document can still show one. [ADR 0027](docs/adr/0027-a-region-inside-a-line.md).

**Code files take regions in their own comments.** `// computed tree src=. depth=1 as=comment` in Rust or TypeScript, `# computed …` in Python, YAML or a Makefile, `/* computed … */` in CSS. `as=comment` writes the text as comment lines, so a crate's `//!` doc can list its modules. With no paths, discovery still reads only Markdown; `[discover] extensions = ["rs"]` in the repository root's `computed.toml` adds code. [ADR 0026](docs/adr/0026-a-marker-is-a-comment-in-the-files-own-syntax.md).

Relative paths in a marker resolve against the directory of the file that contains the marker, not the repository root and not the shell's working directory, and an exec command runs there. A region reads the same from a pre-commit hook, from CI, and from a terminal, and moving the file moves its regions with it. [ADR 0004](docs/adr/0004-region-root-is-the-template-directory.md).

Every loader's text is normalised before a sink sees it, and exec runs with `LC_ALL=C`, `TZ=UTC` and an empty `LANGUAGE`, so the same inputs give the same bytes on the developer's machine and in CI. [ADR 0009](docs/adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md).

### Exec regions run only in a trusted clone

Cloning a repository should not execute anything in it. An exec or transcript region runs only after `computed trust` has recorded a grant for that repository root on this machine, in `~/.config/computed/trust.toml`, never in the working tree. Until then `run` skips the region, keeps its body, reports it `untrusted` and exits 1. Other regions in the same file still render. CI passes `run --trust` for one invocation; a `check`-only pipeline needs no trust at all. [ADR 0007](docs/adr/0007-exec-trust-per-clone.md).

A `remote` region has a gate of its own. `check` never fetches: the snapshot is the url and its pin. `run` fetches only under a url prefix this machine has allowed with `computed allow`, and a body that no longer matches the pin keeps the old one. `computed update` moves the pins, as a reviewable one-line diff per region. [ADR 0019](docs/adr/0019-remote-regions-are-pinned-and-allowlisted.md).

### Keeping `check` honest

`check` answers from sums, so it believes two things it cannot see: that a loader prints the same for the same inputs, and that `inputs=` names everything a command reads. Three tools test them.

- `computed doctor` renders every region twice, the second time from another directory under a changed environment, and reports regions whose output differs, or whose output moved while their inputs did not.
- `computed trace` runs each exec command under a file-access tracer, lists the files it read that `inputs=` does not declare and the declared ones it never read, and with `--write` fixes the opener. It needs no root: `strace` on Linux, the sandbox's own read reports on macOS. [ADR 0021](docs/adr/0021-trace-reads-the-macos-sandbox-reports.md).
- The `sandbox` flag on an exec region runs its command where it can read only its declared inputs and the system's programs, and reach no network. An undeclared read fails the region, so its `inputs=` is complete or it does not render. A sandboxed region still needs trust. [ADR 0020](docs/adr/0020-the-sandbox-enforces-inputs-and-does-not-replace-trust.md).

## The command line

```
computed run      [paths] [--force] [--dry-run] [--trust] [--only NAME] [--allow PREFIX]
computed check    [paths] [--only NAME]
computed clean    [paths] [--force] [--dry-run] [--only NAME]
computed trust    [path]
computed untrust  [path]
computed update   [paths] [--dry-run] [--only NAME] [--allow PREFIX]
computed allow    [PREFIX]
computed disallow PREFIX
computed doctor   [paths] [--only NAME] [--trust] [--allow PREFIX]
computed trace    [paths] [--only NAME] [--trust] [--write]
computed affected PATHS...
computed graph    [paths] [--format mermaid|dot|json]
computed why      FILE [--only NAME | --line N]
computed stats    [paths]
computed dupes    [paths] [--min-lines N]
computed adopt    FILE [--only NAME] [--dry-run]
computed merge    BASE OURS THEIRS [PATH] | --install
computed guard    FILE --proposed PATH | --hook pre|post
computed watch    [paths] [--trust] [--allow PREFIX]
computed lsp
```

`run`, `check` and `clean` are the tool; the rest are built on them. `update`, `allow` and `disallow` move remote pins and keep the allowlist. `doctor` and `trace` test what `check` has to believe. `affected` lists the regions a path reaches, `graph` draws what every region reads, `why` explains from git history why a region is stale, `stats` says how much of each file is computed, and `dupes` finds blocks copied between Markdown files and the `file` region that would replace each copy. `adopt` writes a hand edit in a `file` region back into its source. `merge`, `guard`, `watch` and `lsp` are for git, agents and editors, below.

With no paths, the current directory is walked with the tree loader's ignore settings, dot-directories such as `.claude/` and `.github/` included, and every `.md` and `.markdown` file is read, with the extensions and names a `[discover]` table adds. An explicit file is read whatever its extension, in the comment syntax its name selects. A symlinked file is written through, never replaced. `--only NAME` narrows a command to the regions with that name.

| Exit | Meaning |
|---|---|
| 0 | Nothing to report. Everything is fresh, or only stale where the opener says `on-stale=warn`. |
| 1 | The content said no: drift under `check`; a write, a refused file, a loader failure, or an untrusted or disallowed region under `run`. |
| 2 | The tool could not answer: usage error, marker parse error, a path escaping the repository, `inputs=` matching nothing, a slice that finds nothing, a file edited while `run` computed it. |

One line per region goes to stderr; `--dry-run` diffs are the only thing on stdout. Fresh regions print only with `-v`. `--format json` prints one JSON document on stdout instead, every region included. Each command's own output and exit codes are in the [spec](docs/spec/computed-v0.md#what-is-the-command-line).

### Hooks

Pre-commit runs `run`, CI runs `check`. The committed [`.pre-commit-config.yaml`](.pre-commit-config.yaml) sets `always_run` and `pass_filenames: false`, because a staged change under `src/` makes a region in `CLAUDE.md` stale without staging `CLAUDE.md`. Without the framework, `.git/hooks/pre-commit` is two lines:

```sh
#!/bin/sh
exec computed run
```

The rest is set out in [`docs/integrations.md`](docs/integrations.md):

- **A GitHub Action.** `uses: mitchellvanw/computed-files@main` installs a checked release and runs `check`, annotating each region that is not fresh. With `command: suggest` it posts, on a pull request, what `run` would write as review suggestions.
- **A merge driver.** Two branches that each re-render a region conflict inside it. `computed merge --install` routes Markdown through a driver that takes the side that changed a region and leaves a region both sides changed unrendered for the next `run`. [ADR 0024](docs/adr/0024-the-merge-driver-leaves-doubly-rendered-regions-unrendered.md).
- **Claude Code hooks** that refuse an edit to a region before it lands, with or without the plugin.
- **Editors.** `computed lsp` shows each region's state as you type and renders it from a code lens. Setups for Neovim, Helix and VS Code.
- **`computed watch`**, which runs `run` whenever something a region reads changes.

## Try it

From a clone of this repository, against its own `CLAUDE.md`:

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
src/marker.rs     the marker grammar in every comment syntax, inline regions: parse to prose and regions, serialise back
src/sink.rs       normalisation, raw, fence, table and comment, and the max-lines cut
src/render.rs     the sums, the states, refuse, untrusted, failure, clean; pure behind a Loaders seam
src/fs.rs         the walk, the repository root, the atomic write
src/loader.rs     every loader behind one enum, the pinned shell, the read set, recipe expansion
src/project.rs    the projections: lines=, section=, anchor=, key=
src/trust.rs      the per-clone grant store
src/allow.rs      the per-machine allowlist for remote regions
src/report.rs     the stderr line per region, the dry-run diff, the JSON document
src/cli.rs        clap, discovery, exit tiers, settling, one dispatch arm per command

src/{symbol,git,remote,transcript,index,toc,table,config}.rs
                  one loader, sink or computed.toml each
src/{update,doctor,trace,affected,graph,why,stats,dupes,adopt,merge,guard,watch,lsp}.rs
                  one command each

tests/render.rs   render through a fake Loaders against golden files; the goldens pin the sum vectors
tests/cli.rs      the pre-commit scenario and the exit tiers, end to end
tests/*.rs        one end-to-end file per loader and command

action.yml        the GitHub Action; its scripts are under scripts/action/
claude-code-plugin/  the skills and the guard hooks
docs/spec/        the specification
docs/adr/         the decisions, argued
docs/integrations.md  the Action, the merge driver, the Claude Code hooks, editors
docs/research/    prior-art survey, gitignore semantics for the tree loader
prototypes/       the HTML logic prototypes the design was tested on
CONTEXT.md        the vocabulary: template, region, marker, sum, snapshot, drift
```

`render` is pure: it takes a parsed file, a mode, a trust flag and a `Loaders` seam and returns what to write and a report per region. The production loaders and a table-driven fake both sit behind the seam, so every rule about what a region becomes is tested without a shell or a directory walk. [ADR 0008](docs/adr/0008-render-is-pure-behind-a-loaders-seam.md).

## Not yet

Not built, by decision: a copy layout, colour, Windows, TypeScript for `symbol`. The spec lists what is still open with the reason each waits.

## Vocabulary

The words in this README are chosen on purpose. A region, not a block. A marker, not a directive. A sum, not a checksum. Drift, not stale, for a file as a whole. `CONTEXT.md` lists each term with the words it replaces.
