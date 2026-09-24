# computed

`computed` keeps marked regions of a markdown file current by computation. The document is a view; the truth lives in a directory listing, a file, the repository's history, a pinned document or the output of a command. This is the specification of the tool as it stands on the main branch: the marker grammar, the loaders and sinks, the freshness model, the hand-edit policy, the trust model and the allowlist, the command line, and the crate layout. It began as the design for v0 and milestone 1, and the file keeps that name: the tool is still before 1.0, and nothing here is a stability promise.

Every decision here was made on a ticket and, where it is hard to reverse, recorded as an ADR under [`docs/adr/`](../adr/). The spec gists; the ADR argues. Vocabulary is [`CONTEXT.md`](../../CONTEXT.md), and this document uses its terms without redefining them. Anything the spec leaves open is listed at the end, with the reason it is open.

## What is it for?

Two users: a developer, on macOS or Linux, and a CI job. One dogfood target: this repository. The foundation is `run` in a pre-commit hook and `check` in CI. Everything else is built on those two: a watcher, an editor's language server, a guard on an agent's edits, a merge driver, and commands that explain a region or test whether its declaration is honest.

The problem, stated once: every surveyed tool that rewrites a marked region remembers what it wrote, not what it read. To answer "is this file fresh" they run every generator again, which is the slow path and, for shell regions, the untrusted one. `computed` stores both an input sum and an output sum in the file, so `check` answers from the file and its inputs alone ([ADR 0002](../adr/0002-two-sum-closer.md), [ADR 0006](../adr/0006-check-never-runs-a-loader.md)).

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

A marker is a whole line: optional leading whitespace, which is preserved, the comment, optional trailing whitespace, which is ignored. No other container is a marker: not a link, not inline code, not a fence. Markers inside CommonMark fenced code blocks (backtick or tilde, any length, with a matching closer) are ignored, so a document can show marker examples. Indented code blocks are not tracked. The comment end may touch the last token: `<!-- /computed-->` is a closer.

The opener:

```
<!-- computed <loader> [flag ...] [key=value ...] [| do not edit; run computed] -->
```

- `computed` is lowercase and case-sensitive. Tokens are separated by any run of spaces or tabs. The tool writes canonical single-space form.
- The first bare word is the loader. Later bare words are boolean flags.
- Attributes are `key=value`. A value is unquoted, or double-quoted when it contains whitespace or `>`. The only escape is `\"`. A value may not contain `-->`.
- Each loader owns its attribute and flag set. The common attributes, which every loader takes, are in [Common attributes](#common-attributes).
- The suffix `| do not edit; run computed` is written by the tool into the rendered opener and stripped by the parser. A template does not need it.

The closer:

```
<!-- /computed [in=<hex> out=<hex>] -->
```

`in` is the input sum, `out` the output sum. Both present or neither. A closer with neither means the region is unrendered. A closer with one is a parse error.

The body is the lines strictly between the marker lines. The tool writes what the sink produced and adds no blank lines of its own; the sink emits the blank lines CommonMark needs. When the opener is indented, as inside a list item, every non-blank body line carries the same indentation, so the region stays inside its container. Every line the tool writes into a region ends as the opener's line does, so a CRLF file stays CRLF.

### Parse errors

All hard, the file is not written, exit 2:

- **Structure.** Opener without closer, closer without opener, opener inside a body (nesting is not supported), one-sum closer, malformed sum.
- **Tokens.** Unknown loader, unknown attribute or flag, a duplicate attribute or flag, duplicate name, value containing `-->`, an empty entry in a comma-separated list (a stray comma).
- **Values.** `timeout=0`; `max-lines=0` or a non-number; `on-stale=` other than `warn`; `as=` naming no sink; `delim=` or `from=` without `as=table`, or with a value the sink does not know; a malformed projection, or two projections on one `file`; a projection on a wildcard path in `inputs=`; `min=` greater than `max=` on `toc`; `title=` other than `h1` or `filename` on `index`; a malformed `item=` or `part=` on `symbol`; a `sha256=` that is not 64 lowercase hex digits; a `url=` that is not `https://`, or plain `http://` to a loopback host.
- **Required attributes.** `exec` without `cmd=`, `file` without `src=`, `value` without `src=` or `key=`, `index` without `src=`, `symbol` without `src=` or `item=`, `remote` without `url=`, `transcript` without `steps=`, `use` without `recipe=`, and `git` without exactly one of `log`, `tags` or `contributors`.
- **Combinations.** Neither or both of `inputs=` and `volatile` on `exec` or `transcript`; `sandbox` with `volatile`; `sandbox` with `workdir=copy`; a loader attribute or flag on a `use` opener.

### Why two sums?

[ADR 0002](../adr/0002-two-sum-closer.md). One sum over the output, cog's model, detects hand edits but cannot tell whether the inputs moved. Sums in a sidecar are a second file to lose. Two sums in the closer let a fresh clone carry everything `check` needs.

## Which layout?

In-place only. The template and the rendered file are one file. There is no file-level banner; the opener suffix is the generated signal, true at the granularity it claims ([ADR 0003](../adr/0003-in-place-layout.md)).

Writes go through a temp file in the same directory and `rename(2)`, with the original's mode bits copied. When the rendered bytes equal the current bytes nothing is written and the mtime is untouched, so `run` twice is a no-op.

Copy layout, a `.tmpl` rendered to the canonical path, was the research recommendation and lost: it reverts every prose edit made to the rendered file, which on an agent-edited `CLAUDE.md` is the normal case. The parser, render and sum core stay layout-agnostic so copy can return as an opt-in.

## Which loaders?

Ten, and a recipe that stands for any of them. Modelled as a closed `enum Loader`, one variant per loader, so an enum and not a trait. All produce the same thing, `Loaded { text: String, snapshot: Vec<u8> }`: native loaders and commands are one kind of thing at the data level. The set grows by native variants, not by a plugin interface ([ADR 0017](../adr/0017-trust-gates-running-repository-code-and-nothing-else.md)).

| Loader | Renders | Snapshot | Trust | Default sink |
|---|---|---|---|---|
| `tree` | a directory listing | the listed paths | no | `fence` |
| `exec` | a command's stdout | the declared `inputs=` | yes | `raw` |
| `file` | one file, or a slice of it | that file or slice | no | `raw` |
| `value` | one scalar of a TOML, JSON or YAML file | the scalar | no | `raw` |
| `index` | a linked list of files matched by globs | each path and its title | no | `raw` |
| `toc` | the template's own headings | each listed heading and anchor | no | `raw` |
| `symbol` | one item of a Rust, Python or Go file | the extracted text | no | `fence` |
| `git` | recent commits, tags or contributors | commit ids, or the names | no | `raw` |
| `remote` | a document fetched over HTTPS | the url and its pin | no; the allowlist | `raw` |
| `transcript` | a shell session, prompt and output | the declared `inputs=` | yes | `fence` |
| `use` | whatever its recipe expands to | the expansion's | the expansion's | the expansion's |

Every loader has a format constant, all 1 today.

### Paths and the region root

Every relative path in a marker resolves against the template's directory, the region root, and an exec command or a transcript runs with that directory as its working directory ([ADR 0004](../adr/0004-region-root-is-the-template-directory.md)). Neither the repository root nor the invocation directory: the first makes a nested file spell its own path back, the second makes the same file render differently from a hook, from CI and from a shell.

Every `src=` and `inputs=` must resolve inside the repository root, or inside the region root when the file is not in a repository. A path that escapes is a hard error. This is a reproducibility rule, not a security fence: a region reading outside the repository renders differently on every machine.

### Common attributes

- `name=` is optional and unique per file. Reports use the name, else `loader@line`.
- `as=` selects the sink: `raw`, `fence` or `table` ([Which sinks?](#which-sinks)). `lang=` sets the fence language, default empty, except where a loader sets its own.
- `max-lines=N`, at least 1, cuts the loader's text to its first N lines and a note, `… K more lines` ([What makes loader text deterministic?](#what-makes-loader-text-deterministic)).
- `on-stale=warn` lets `check` report the region `stale` without failing ([When is a region fresh?](#when-is-a-region-fresh)).

All four are part of the canonical opener, so changing one makes the region stale.

### Projections

A projection names the part of a file a region reads, so the snapshot holds that part and an edit elsewhere leaves the region fresh ([ADR 0016](../adr/0016-a-projection-snapshots-only-the-part-it-reads.md)). Four kinds:

- `lines=A-B`: 1-based and inclusive. `A-` runs to the end, `A` is `A-A`. A range past the end of the file is an error, not clamped.
- `section=TEXT`: the first ATX heading, any level, whose text equals TEXT as written; the slice runs to the next heading of the same or a higher level, heading line included. Headings in fences and front matter do not count. Setext headings are not read.
- `anchor=NAME`: the lines between a line containing `ANCHOR: NAME` and the next containing `ANCHOR_END: NAME`, both excluded, and other anchor lines dropped, mdBook's convention.
- `key=a.b.c`: a dotted path into a `.toml`, `.json`, `.yaml` or `.yml` file, numeric components indexing arrays. As a slice it is the value in canonical compact JSON with table keys sorted, so reordering the file or editing comments is not a change.

`file` takes the first three, `value` takes `key=`, and an exec or transcript `inputs=` entry takes any of them as a suffix on a literal path: `inputs="Cargo.toml#key=package.version,src/cli.rs#lines=40-90,docs/*.md"`. The entry splits at the first `#` followed by a projection kind. A projection that finds nothing is a hard error for its region, and every message starts with the canonical projection.

The snapshot entry for a projected read is `path#<canonical projection>` `\0` decimal byte length `\0` the projected bytes `\0`, taken after closer sums are removed, sorted with the plain entries by key.

### `tree`

- `src=` default `.`, and must be a directory. `depth=` default unlimited, counted as `tree -L n` counts. Flags: `all` includes dotfiles, `dirs` lists directories only.
- Output in `tree`'s box-drawing style with a `.` root line. No sizes, mtimes or counts.
- Gitignore-aware through the `ignore` crate, configured as the research settled ([ignore semantics](../research/ignore-gitignore-semantics.md)): `.hidden(true)`, `.ignore(false)`, `.git_ignore(true)`, `.parents(true)`, `.require_git(true)`, `.git_exclude(false)`, `.git_global(false)`, `.follow_links(false)`, byte-order sort by file name, the sequential walker. Per-clone and per-user exclude files are off because they would make the snapshot differ between machines. `tree --gitignore` is not git-exact and is not a byte-for-byte reference. The rules apply inside a repository without any flag, and not at all outside one; `gitignore` is not a flag ([ADR 0011](../adr/0011-gitignore-is-not-a-flag.md)).
- One walk, byte order of names, directories and files interleaved. The rendered listing and the snapshot are the same sequence. A directory whose children are all ignored is still listed.
- Snapshot: one relative path per line, LF-terminated, directories with a trailing `/`, limited by `depth`, `all` and `dirs` exactly as the listing is.

### `exec`

- `cmd=` required. Run as `/bin/sh -c "<cmd>"`, never the login shell. stdin is closed.
- Exactly one of `inputs=` or the `volatile` flag. Neither, or both, is a parse error.
- `inputs=` is comma-separated globset syntax: `**`, `*`, `?`, `[..]`, no braces. Inside a repository, a wildcard component does not select a path the `.gitignore` files ignore, read as the `tree` loader reads them, and never selects `.git`; a literal component reaches what it names, ignored or not, so `target/*.json` selects files in an ignored `target/` and `**/*.json` does not. A directory the glob matches brings every file under it that its own rules do not ignore ([ADR 0012](../adr/0012-wildcards-in-inputs-do-not-reach-ignored-paths.md)). A trailing `/` names the directory the path without it names. Paths are matched as the glob spells them, so `docs/*.md` works when `docs` is a symlink. A symlink to a file is read through when its target stays inside the repository; a symlink to a directory is entered only when a literal component names it. A target outside the repository is an error when named and skipped when a wildcard reached it. Expansion enters only the directories a leading run of the glob's components can match, so `*.md` reads the region root's listing and nothing below it, and a literal path reads its parent's. A file or directory deleted while expansion lists or reads it is left out, not an error. A glob that matches nothing is a hard error under `run` and under `check`, and says so when ignore rules are why. The template file itself is silently excluded from its own snapshot.
- A literal entry may carry a projection ([Projections](#projections)). A projected entry that names a missing path, a directory or the template itself is a hard error.
- Snapshot with `inputs=`: for each matched file in byte-order sorted relative path, `path` `\0` decimal byte length `\0` content `\0`, where the content has the `in=`/`out=` sums taken out of every closer line ([ADR 0014](../adr/0014-snapshots-ignore-sums-and-run-settles-across-files.md)). Projected entries as above. `volatile`: an empty snapshot.
- `timeout=` in seconds, at least 1, default 30. Expiry kills the process group and counts as failure. When the shell exits, the process group is killed too: the output is what the command printed before its shell was done, so a background job cannot hold the pipes open. A process that left the group and still holds them is a failure once the timeout has passed.
- Environment: the inherited environment with `LC_ALL=C`, `LANGUAGE=` (empty) and `TZ=UTC` set unconditionally, plus `COMPUTED_FILE` (the template's absolute path, whatever directory the tool was invoked from), `COMPUTED_ROOT` (the repository root, unset outside one) and `COMPUTED_REGION` (name or `loader@line`). Nothing else is touched, `PATH` included. A command that wants a locale or zone sets it inside `cmd=` ([ADR 0009](../adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md)).
- stdout must be UTF-8; otherwise the loader failed.
- The `sandbox` flag runs the command in an operating-system sandbox that allows reading only the declared input files and the system's program trees, listing directories inside the repository, writing only a fresh `TMPDIR` and `/dev/null`, and no network. An undeclared read fails the command, so the region fails. It needs `inputs=`, still needs trust, and where no sandbox is available the region is an `error`, never run unsandboxed. macOS uses `sandbox-exec`; Linux uses Landlock (5.13 or later) and a seccomp filter ([ADR 0020](../adr/0020-the-sandbox-enforces-inputs-and-does-not-replace-trust.md)).

### Loader failure

Non-zero exit, timeout, invalid UTF-8, or text that fails normalisation (below): the previous body is kept, the sums are kept, the command's stderr is reported under the region's line, nothing is written into the region, and the run exits 1. Deleting an input does not blank a region; restoring it and running again repairs it. Every loader fails this way; the native ones fail on unreadable or malformed data, and `remote` on a fetch or a pin mismatch.

### `file`

A verbatim include that runs no command and needs no trust ([ADR 0015](../adr/0015-the-file-loader.md)).

- `src=` required, a file inside the repository, not the template itself.
- At most one of `lines=`, `section=` or `anchor=` takes a slice ([Projections](#projections)).
- Text: the file's content with closer sums taken out, then the slice. It must be UTF-8; otherwise the loader failed, or for `section=` and `anchor=` the region is an error.
- Snapshot: the one entry `inputs=src` would take, or `src#<projection>` with the slice.
- `as=fence lang=…` shows the file as code, markers included.

### `value`

- `src=` and `key=` required: `value src=Cargo.toml key=package.version`.
- Text: the scalar at the key, strings without quotes, TOML numbers and dates as TOML prints them, JSON numbers in serde's canonical form, YAML scalars as written, booleans, `null`.
- A table, an array, a multi-line value, a missing key, a file that does not parse or is not UTF-8, and an unknown extension are hard errors.
- Snapshot: `src#key=a.b.c` `\0` length `\0` the scalar text `\0`.

### `index`

- `src=` required: comma-separated globs, expanded exactly as `inputs=` expands them. A glob that matches nothing is a hard error. `title=h1` (default) or `title=filename`.
- Text: one line per file, `- [Title](path)`, in byte order of the path relative to the region root, the template left out. Space, `(`, `)`, `<`, `>` and `%` are percent-encoded in the link.
- The title is the first level-one ATX heading of a `.md` or `.markdown` file, outside fences and front matter, else the file name.
- Snapshot: per file, `path` `\0` length `\0` title `\0`, so a body edit that keeps the title keeps the region fresh.

### `toc`

- `min=` and `max=`, levels 1 to 6, default 2 and 3. A bound given alone moves the other out of its way.
- Text: a nested list, two spaces per level, `- [Heading](#anchor)`, of the template's own ATX headings in prose. Every region body is left out, the toc's own included, so rendering never moves it. Anchors follow github-slugger, duplicates numbered `-1`, `-2`.
- Snapshot: per listed heading, its level, text and anchor. The template is not in the read set: a run's own write leaves its prose unchanged.

### `symbol`

- `src=` and `item=` required, `part=whole|signature|doc`, default `whole`. The grammar comes from the extension: `.rs`, `.py`, `.go`, parsed with tree-sitter compiled into the binary.
- `item=` is `name`, `a::b::name` (Rust inline modules), `Type.member`, or `"Trait for Type.member"` for Rust. No match, or more than one, is a hard error, and the ambiguity names the candidates and their lines.
- `whole` is the item with its attached docs, attributes or decorators, dedented. `signature` is the item cut before its body. `doc` is the doc text without comment markers; an item with none is a hard error.
- Default sink `fence` with `lang=` from the extension; `part=doc` defaults to `raw`.
- Snapshot: `src#item=…,part=…` `\0` length `\0` the extracted text `\0`, so an edit elsewhere in the file keeps the region fresh.

### `git`

- One query flag: `git log [src=PATH] [n=N]`, `git tags [n=N]`, `git contributors [src=PATH]`. `src=` defaults to `.`; `n=` to 10.
- Text: a list. `log`: `- <7 hex> <subject>`, newest first. `tags`: `- <tag>`, newest first. `contributors`: `- <author>`, mailmapped, in order of first commit.
- Snapshot: the full commit ids for `log`, the names in order for the others. The snapshot runs the same `git` query, so `check` runs `git` ([ADR 0018](../adr/0018-the-git-snapshot-runs-git-under-check.md)). Reading history runs no repository code and needs no trust.
- `git` runs in the region root with the configuration keys that could start a program or reshape the output overridden, and the `GIT_*` variables that redirect it removed.
- Outside a repository, in a shallow clone, with no commits yet, or with `src=` missing from the working tree: a hard error.

### `remote`

- `url=` required: `https://`, or `http://` to a loopback host. `sha256=` pins the body; `timeout=` in seconds, default 30, across all redirects.
- Snapshot: the url and the pin. `check` never touches the network or the allowlist.
- `run` fetches only when every url, redirects included, is on this machine's allowlist ([Who may fetch a url?](#who-may-fetch-a-url)); otherwise the region is `disallowed`, keeps its body, and exits 1. With no pin it fetches nothing and fails, saying to run `computed update`. A body that does not match the pin is a loader failure. Bodies are capped at 10 MiB and must be UTF-8 ([ADR 0019](../adr/0019-remote-regions-are-pinned-and-allowlisted.md)).

### `transcript`

- `steps="STEP ;; STEP ;; …"` required, split on ` ;; `. Exactly one of `inputs=` or `volatile`, as exec. `timeout=` for the whole session. `workdir=tmp` or `workdir=copy`. The `sandbox` flag, as exec, except with `workdir=copy`.
- The steps run in one `/bin/sh`, so a `cd` or a variable carries to the next, under exec's pinned environment, process group and timeout. Text: each step as `$ <step>`, then its stdout, then its stderr. A step that exits non-zero is part of the transcript; a shell that ends before a step finishes is a loader failure.
- Default: the region root. `workdir=tmp`: a fresh empty directory. `workdir=copy`: a temporary copy of every file the walk lists under the repository root, with an empty `.git`, so the steps can change files without touching the original.
- Snapshot as exec. Needs trust, as exec does. Default sink `fence`, `lang=console`.

### Recipes: `use` and `computed.toml`

`use recipe=NAME` stands for an opener named in `computed.toml` ([ADR 0022](../adr/0022-recipes-in-computed-toml.md)):

```toml
[recipe.adrs]
loader = "index"
src = "docs/adr/*.md"
```

- The file holds `[recipe.NAME]` tables and nothing else. Each has `loader`, the loader's attributes as strings or integers, flags as `true`, and any of `as`, `lang`, `on-stale` and `max-lines`. A recipe cannot use another recipe, and cannot set `name`.
- A `use` opener takes `recipe=` and common attributes, which override the recipe's. A loader attribute or flag on it is a parse error.
- Lookup: the nearest `computed.toml`, walking up from the region root to the repository root, never above it. Read only for a template with a `use` region.
- The expansion is the region's opener for every purpose. Paths resolve against the using template's region root. The input sum is over the expanded canonical opener, so a `use` region has the sums of the equivalent inline opener and switching between the two is not a change. Editing the recipe is. An exec recipe needs trust.
- `computed.toml` is in the read set. A missing file or recipe, or any error in the file, makes the regions that use it `error`; only those are skipped. The file shows the `use` opener as written.

## Which sinks?

- **`raw`** writes a blank line, the text, a blank line.
- **`fence`** writes the opening fence with `lang=`, the text, the closing fence. The backtick run is one longer than the longest that starts a line of the text, minimum three.
- **`table`** reads the text as CSV (`as=table`, RFC 4180 quoting), tab-separated values (`delim=tab`, no quoting) or JSON Lines (`from=jsonl`, columns from the first object's keys) and writes a GitHub Markdown table: a blank line, a header row, a delimiter row, the data rows padded to the widest cell, a blank line. A newline in a cell becomes `<br>` and an unescaped `|` becomes `\|`. A ragged row, bad JSON or empty text is a loader failure.

Every line is LF-terminated. Interior blank lines are untouched.

## What makes loader text deterministic?

The output sum is taken over body bytes, so anything that changes those bytes for the same inputs is drift. Two sources of change are not inputs at all: the shape of the text a command prints, and the environment it prints under. Both are fixed before a sink sees the text ([ADR 0009](../adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md)).

Normalisation, applied in the `sink` module to every loader's text before any sink shapes it, in order:

1. Invalid UTF-8, or any C0 control byte other than tab, LF and CR: loader failure.
2. CRLF and lone CR become LF.
3. Trailing newlines are stripped. Trailing spaces and tabs on a line are kept: markdown gives them meaning and padded tables carry them. Empty output stays empty.

Then `max-lines=N` cuts text longer than N lines to its first N and one note line, `… K more lines` (or `… 1 more line`), which cannot parse as a marker or open a fence. `raw` and `fence` keep the note as the text's last line. `table` keeps the header, cuts data rows, and puts the note in its own paragraph after the table. The output sum is over the cut body.

One rule keeps the file parseable, because the grammar skips markers inside fences and loader text can contain fence lines: after the sink has shaped the body, the region must parse back to the same body. `fence` guarantees this by its backtick run. For `raw`, text whose fences are unbalanced would swallow the closer on the next parse; that is a loader failure, and so is a line that would parse as a marker unless a fence in the text holds it. A `max-lines` cut that leaves a fence open fails the same way. Every table line starts with `|`, so a table cannot break the parse. The invariant that a file the tool wrote always parses is then checked on the whole file before it is written, because a region's fence can close a fence the prose left open above it: when the rendered file does not parse back to the same regions, the file is not written, exit 2, and the error names the unclosed fence.

A change to any normalisation rule bumps every loader's format constant.

## When is a region fresh?

A region carries two sums in its closer. Both are SHA-256, stored as the full 64 lowercase hex characters ([ADR 0010](../adr/0010-sha-256-sums.md)).

**Input sum.** SHA-256 over, in order: the domain line `computed-in/1\n`; the loader and its format constant, `<loader>/<n>\n`, or `<loader>/<n> indent="<indent>"\n` for an indented opener, a tab written `\t`; the canonical opener line (single-space tokens, suffix stripped, indentation stripped) followed by `\n`; the snapshot bytes. For a `use` region the loader and opener are the expansion's.

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
| `error` | The snapshot was a hard error, so the tool could not answer ([ADR 0013](../adr/0013-a-region-the-tool-cannot-answer-skips-only-itself.md)). |

A volatile region whose body does not match `out=` is `edited`. Volatile exempts only the input side of the test, so a hand edit inside a volatile body is still caught.

**`on-stale=warn`.** Under `check`, a region that is exactly `stale` and says `on-stale=warn` is reported with the action `warn` and does not raise the exit code. Every other state fails as before, and `run` renders it as any stale region ([ADR 0023](../adr/0023-on-stale-warn-softens-only-staleness.md)).

**The cache.** `run` skips the loader of a fresh region. A loader therefore has two steps: `snapshot`, computed before any work and always run by both `run` and `check`, and `load`, run only when the region is stale, unrendered, or volatile. For `tree` and `git` both steps are one query. Fresh regions are reproduced byte-for-byte from their raw lines: canonical spacing and the opener suffix appear the first time a region renders, so a clean `check` guarantees `run` is a no-op.

**`check` never runs a loader's `load` step, and never a command from the repository** ([ADR 0006](../adr/0006-check-never-runs-a-loader.md), amended by [ADR 0018](../adr/0018-the-git-snapshot-runs-git-under-check.md)). It computes snapshots, compares both sums and reports states. Snapshots read files and, for `git` regions, run `git` to read history; they never run `exec` or `transcript`, and never fetch. So `check` is safe on an unvetted clone and cheap in a hook, and it cannot show the diff `run` would write. That diff lives on `run --dry-run`. The cost, accepted: a loader whose output changes without an input or format-constant change is invisible to `check`. `doctor` finds such a region and calls it moved; `run --force` renders fresh regions too.

## What happens to a hand edit?

`run` refuses ([ADR 0005](../adr/0005-refuse-hand-edited-regions.md)). A region is edited when its body does not match `out=`. Only the body is subject to this test.

- **Default.** The file is not written, the region is named (file, line, `name=` when present), and the invocation exits 1. There is no policy switch.
- **`--force`.** `run --force` overwrites every edited region in every file the invocation processes, and renders fresh regions too, so output that moved without its inputs is caught up. Narrow the scope by passing paths or `--only`. `check --force` is a usage error.
- **Per file.** A file with one edited region is left untouched in full, merely stale regions included. Other files in the same invocation are rendered and written. The exit code is 1 when any file was refused.
- **Openers and closers never reach this policy.** An edited opener changes the input sum: the region is `stale` and re-renders. A closer missing both sums is `unrendered` and renders whatever the body contains. Any other closer damage is a parse error and nothing in the file is written.

Prose outside a region needs no policy. After a prose edit `run` renders the identical file and skips the write.

Why refuse, when every tool but cog overwrites? The hand-edit prototype ([`prototypes/hand-edit.prototype.html`](../../prototypes/hand-edit.prototype.html)) showed the one case that matters: a body edit and an input change landing together. Under overwrite the edit vanishes inside a legitimate re-render, indistinguishable from a normal run. Under warn the hook writes and exits 0, so on the pre-commit path it is overwrite with a line nobody reads. On `CLAUDE.md` the editor is usually an agent that does not know the region is owned, and a failed hook is the one signal that reaches it.

Two commands meet the edit earlier or keep it. `guard` refuses an agent's edit to a body or closer before it lands ([ADR 0025](../adr/0025-the-guard-refuses-an-edit-before-it-lands.md)). `adopt` writes an edit inside a `file` region back into the file it copies, when the edit round-trips.

## Who may run a command?

An exec or transcript region runs only when the repository it sits in has been trusted on this machine ([ADR 0007](../adr/0007-exec-trust-per-clone.md)). The model is direnv's and git's `safe.directory`. Trust gates running a command the repository controls, and nothing else: every native loader, `git` and `remote` included, renders without it ([ADR 0017](../adr/0017-trust-gates-running-repository-code-and-nothing-else.md)).

**What it defends against.** One thing: cloning a repository and having `computed run` execute its commands before anyone read them. A malicious branch pulled into a trusted clone, or a dependency writing markers into a file, runs on the next `run` exactly as a Makefile or a pre-commit hook would. We state this as accepted rather than imply a guarantee the model cannot keep. The `sandbox` flag does not change it: a sandboxed region still needs trust.

**Grant.** `computed trust [path]` records a grant for the repository root containing `path` (default: the current directory), found by walking up for `.git`; outside a repository, the directory itself. It prints the root it recorded. `computed untrust [path]` removes it. The store is `$XDG_CONFIG_HOME/computed/trust.toml`, default `~/.config/computed/trust.toml`, on every platform, one entry per canonical root with symlinks resolved. Nothing inside the working tree can grant trust. A grant covers the root it names and every path under it, compared by path component and never by string prefix, so `/a/b` covers `/a/b/c` and not `/a/bc`.

**One shot.** `--trust` on `run`, `run --dry-run`, `doctor`, `trace` and `watch` treats every file in the invocation as trusted without writing the store. This is how CI expresses trust. There is no environment variable. A `check`-only pipeline needs no trust at all.

**Lookup.** Per template file, against that file's own repository root (the same value as `COMPUTED_ROOT`), or its region root outside a repository. That root is its own: a linked worktree's `.git` is a file, so a worktree laid down inside a clone answers with itself, and so does a submodule. Neither asks for a grant of its own, since a grant on the clone that holds it covers it.

**Untrusted.** `run` skips every exec and transcript region: body kept, nothing written into it, the region reported as `untrusted` with file, line and name, and the run exits 1. Other regions in the same file still render and the file is still written. `check` never runs a command, so trust never enters it.

## Who may fetch a url?

A `remote` region fetches only under a url prefix on this machine's allowlist ([ADR 0019](../adr/0019-remote-regions-are-pinned-and-allowlisted.md)). The allowlist and trust are independent: a grant allows no url, and an allowed url needs no grant.

- **Store.** `remote.toml` beside `trust.toml`, never in the working tree. `computed allow PREFIX` records a prefix and prints its normal form; `computed allow` alone lists them; `computed disallow PREFIX` removes one.
- **One shot.** `--allow PREFIX`, repeatable, on `run`, `run --dry-run`, `update`, `doctor` and `watch`, without writing the store. There is no environment variable. `check` never reads the allowlist.
- **Matching.** Both sides are parsed. Scheme and host compare without case, a default port is dropped, `.`, `..` and `%2e` segments are resolved, and the path matches at a `/` boundary: `https://h/org/` covers `/org/x` but not `/org`, and `https://h/org` covers `/org` and `/org/x` but not `/orgs`. The fetched url's query and fragment are ignored. An entry with credentials, a query, a fragment or a wildcard host is refused, and a url with credentials is never allowed.
- **Disallowed.** `run` skips the region as it skips untrusted exec: body kept, reported `disallowed` with the prefix to allow, exit 1.

## What is the command line?

`--help` and `--version` come from clap. `-v` is global and shows the regions that are otherwise silent. `--format json` is global and prints one JSON document on stdout instead of the report and the diffs; `why`, `merge` and `adopt` are text only and refuse it. `--format mermaid` and `--format dot` apply to `graph` alone. `--only NAME`, repeatable, narrows a command to the regions with that name: the others are left as they are and neither refuse the file nor move the exit code. A name no file has is exit 2.

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
computed merge    BASE OURS THEIRS [PATH]
computed merge    --install
computed guard    FILE --proposed PATH
computed guard    --hook pre|post
computed watch    [paths] [--trust] [--allow PREFIX]
computed lsp
```

A flag a command does not take is a usage error: `check --force`, `check --trust`, `clean --trust`. No `-C`, no stdin except where a command reads a hook's JSON or speaks a protocol, no `-`.

**Discovery.** With no paths, walk the current directory with the same `ignore` settings as the tree loader, dotfiles included (so `.claude/` and `.github/` are covered, and `.git` never is), and read `.md` and `.markdown` files. Symlinks found by the walk are skipped: the file they name is found in its own right. An explicit file is read whatever its extension; an explicit symlink is its target, resolved against the target's directory and written through, never replaced. Two paths to one file are processed once. A file with no marker is skipped whatever its encoding; a file with markers must be UTF-8. An explicit directory is walked. A file with no opener is skipped silently. A path that does not exist is a usage error. Files are processed in byte-order sorted path order. Every command that takes `[paths]` discovers this way.

**Exit codes.**

| Exit | Meaning | Examples |
|---|---|---|
| 0 | Nothing to report. | Everything fresh; `on-stale=warn` regions that are only stale; a query that answered. |
| 1 | The content said no. | Drift under `check`; a refused file, a loader failure, an untrusted or disallowed region under `run`; a file `--dry-run` would have changed; a pin `update` wrote; a region `doctor` found nondeterministic or moved; an undeclared read under `trace`; duplicates under `dupes`; an edit `guard` refused. |
| 2 | The tool could not answer. | Usage error, marker parse error, path escaping the root, `inputs=` matching nothing, a projection that finds nothing, unreadable file, a file that changed on disk while `run` computed it, templates that never settle, no sandbox or tracer on this machine, a fetch that failed under `update`. |

The invocation exits with the highest tier any file or region hit. A parse error skips its file whole. A region the tool cannot answer, such as a hard loader error, skips only itself: it is reported `error` with the message beneath, its body and sums are kept, and the rest of its file is still rendered and written ([ADR 0013](../adr/0013-a-region-the-tool-cannot-answer-skips-only-itself.md)). Other files are still processed and written.

**Settling.** `run` writes a file only when it still holds the bytes it was read as, so an edit made while commands ran is not overwritten. After a pass over the files, `run` passes again over every file whose read set holds a file it just wrote, until a pass writes nothing; one `run` leaves every file it touched fresh under `check`, whatever order they sort in. Files still changing after one pass per file feed each other with no fixed point, and are exit 2 ([ADR 0014](../adr/0014-snapshots-ignore-sums-and-run-settles-across-files.md)).

**`run --dry-run`.** Renders everything `run` would, prints a unified diff per file that would change to stdout, writes nothing. A refused file prints its refusal line and the diff `run --force` would apply, labelled `(run --force)`, so the hand edit can be kept before it is overwritten. Exit code as `run`, plus 1 when any file would have changed. This is the only path that shows the pending diff.

**Reporting.** One line per region on stderr, columns `path:line`, name (blank when absent), loader, state, action:

```
CLAUDE.md:12 layout tree   stale      written
CLAUDE.md:40 deps   exec   edited     refused; run with --force
CLAUDE.md:58        exec   untrusted  skipped; run `computed trust`
NOTES.md:9   rules  remote disallowed skipped; run `computed allow`
```

Fresh regions, and volatile regions under `check`, print only with `-v`. A region softened by `on-stale=warn` prints under `check` with the action `warn`. Nothing is printed and the exit is 0 when everything is fresh. Loader stderr, or the message of a region in `error`, is printed indented under the region's line. A region reported identically by more than one pass is printed once. `check` uses the same shape without the action column. Diffs are the only thing on stdout. No colour.

**JSON.** `--format json` prints `{"exit": n, "files": [...]}` on stdout and nothing on stderr. Each file with something to say has `path`, `error` (`{line, message}` or null, `line` null for the file as a whole), `regions` (every region, silent ones included: `line`, `name`, `loader`, `state`, `action` as a key such as `written`, `would-write` or `disallowed`, or null under `check`, `severity`, `"warn"` for a softened region and null otherwise, and `message`), and `diff` (the `--dry-run` diff or null). The other commands print their own documents, each with an `exit` key.

**`clean`.** Empties every region body and strips both sums from the closer, leaving the region unrendered with its opener line unchanged. Markers stay, so the next `run` rebuilds the region. Runs no loader and needs no trust. Honours the hand-edit policy: an edited region refuses the file, `--force` overrides. Takes paths and `--dry-run` like `run`.

### `update`, `allow`, `disallow`

`update` fetches every `remote` region's url under the allowlist and rewrites the opener's `sha256=` in place, inserting it after `url=` when there is none; the rest of the line is untouched. It renders nothing: the moved pin makes the region stale, and the next `run` fetches and renders it. A matching pin is `fresh` and silent. `--dry-run` prints the diff. Exit 0 when nothing changed, 1 when a pin was or would be written or a region was disallowed, 2 on a failed fetch. For a `use` region whose recipe is a remote, a moved pin is reported for `computed.toml`, tier 2, and not written. `allow` and `disallow` are in [Who may fetch a url?](#who-may-fetch-a-url).

### Is the output honest? `doctor` and `trace`

`check` believes two things it cannot see: that a loader prints the same for the same inputs, and that `inputs=` names everything a command reads. These two commands test them. Both write nothing unless asked.

- **`doctor`** renders every selected region twice, as `run --force` would: once normally, and once from a scratch working directory, under another umask, with each exec command restarted with a changed `HOME`, `TMPDIR`, `USER` and an extra variable. It reports each region `deterministic`, `nondeterministic` (with a short diff), `failed`, `untrusted`, `disallowed` or `error`, and flags a fresh region whose loader now prints something other than its body as moved, with `run --force` as the fix. Silent regions print with `-v`. Exit 1 on anything nondeterministic, moved, failed, untrusted or disallowed; 2 on an error.
- **`trace`** runs each selected exec and transcript command under a file-access tracer and compares the repository files it read with what `inputs=` expands to. Verdicts: `complete`, `undeclared` (a false fresh waiting to happen), `unused` (a false stale), `undeclared+unused`, `volatile` (reads listed, not judged), `failed`, `untrusted`, `error`. It suggests an `inputs=` value, collapsing a directory's files to `dir/*.ext` when that glob selects exactly them, and `--write` puts the suggestion in the opener, which makes the region stale. Commands run unsandboxed. macOS uses `sandbox-exec`'s read reports, read back with `log show`; Linux uses `strace`; with no tracer the command exits 2 before touching a file ([ADR 0021](../adr/0021-trace-reads-the-macos-sandbox-reports.md)). Exit 1 on undeclared reads, failures, untrusted regions or a rewrite.

### Questions about regions

These run the snapshot step at most, never `load`, so they are safe on an unvetted clone.

- **`affected PATHS...`** lists, on stdout, every region a path reaches: through its read set, a `tree` or `file` `src=` at or under the path, an `inputs=` glob that would match it, `git log` or `git contributors` history under it, `computed.toml` for a `use` region, or the template itself for `toc`. Paths need not exist, so a deleted or a new path is a fair question. Exit 0, or 2 when a template does not parse.
- **`graph`** draws templates, their regions and their inputs as Mermaid (default), Graphviz (`--format dot`) or JSON. An input is drawn as the opener names it, a glob as one node. A region that reads another template points at it with a dashed `settles` edge.
- **`why FILE`** explains a region's state from git history. For a stale region it finds the latest commit whose template holds the same `in=` and whose tree, laid down in a temporary directory, recomputes to it, then prints that commit and what changed since: the opener, the listing, each input `added`, `removed` or `changed` (with a diff under `-v`), and the commits that changed them. Exit 2 when no commit reproduces or the file is not in a repository; `why` is a query, and `check` is the gate.
- **`stats`** counts every region's body lines, bytes and estimated tokens (bytes over four, marked `~`), and each file's computed share, on stdout. No loader runs.
- **`dupes`** finds verbatim blocks of at least `--min-lines` lines (default 4) copied between Markdown files outside regions, and suggests for each copy the `file src=… section=…` or `lines=` region that would include the original. Exit 1 when it finds any.

### `adopt`

`adopt FILE` writes a hand edit inside a `file` region back into its source. For each selected region in state `edited`, it takes the opener's indentation and the sink's own lines off the body, splices what is left into the source, over the slice when there is one, and renders the region again. The rendered body must equal the edited one before and after the write, or every source is put back. It refuses other loaders, `stale+edited` regions, a `table` sink, a body cut by `max-lines`, a source that is itself a template, and two regions adopting into one source. `--dry-run` prints each source's diff. Exit 1 on a refusal or when `--dry-run` would write.

### `merge`

A git merge driver for templates ([ADR 0024](../adr/0024-the-merge-driver-leaves-doubly-rendered-regions-unrendered.md)). `computed merge BASE OURS THEIRS [PATH]` takes git's `%O %A %B %P` and writes the result to OURS. It merges the prose and openers line by line, with each region's body and closer held out. A region only one side changed takes that side; a region both sides changed differently takes ours' body under a closer with no sums, so it is unrendered and the next `run` renders it from the merged inputs. Exit 0 when no conflict is left, 1 when one is, 2 on an error. `computed merge --install` adds `*.md merge=computed` and `*.markdown merge=computed` to the root `.gitattributes` and sets the driver in this clone's git config.

### `guard`

Whether an edit changes what the tool owns ([ADR 0025](../adr/0025-the-guard-refuses-an-edit-before-it-lands.md)). An edit may change prose, add a region, remove one whole, move one intact, or change an opener. It may not change a body or a closer, or break the markers.

- `guard FILE --proposed PATH` judges the text in PATH against FILE: exit 0 when allowed, 1 when a region is touched or the markers break, 2 on a usage error.
- `guard --hook pre` reads a Claude Code PreToolUse hook's JSON on stdin, applies the `Edit`, `MultiEdit` or `Write` in memory, and answers `permissionDecision: deny` with a reason when the edit is refused. `guard --hook post` runs `check` on the edited file and returns any drift as `additionalContext`. Both always exit 0.

### `watch`

`run` again whenever something a region could read changes, until Ctrl-C. It watches the named directories and each template's repository root, and a change triggers a pass unless `.gitignore` ignores it; a change to a file in any region's read set always does. The tool's own writes are recognised by their bytes and dropped. Events settle for 200 ms, at most 2 s under continuous change. Each pass prints what `run` prints, one JSON document per pass with `--format json`. A volatile region re-renders on every pass. An error in a later pass is reported and watching continues; one in the first pass exits 2. Ctrl-C exits 0.

### `lsp`

A language server on stdin and stdout for any editor that speaks the protocol.

- **Diagnostics** are `check` of the unsaved buffer against inputs on disk, published on open, change and save, spanning opener to closer. `stale` and `unrendered` are warnings; `edited`, `stale+edited` and `error` are errors; an exec region that is not fresh in an untrusted clone is information.
- **Hover** anywhere in a region shows its loader, state, canonical opener, the `use` opener it expanded from, its read set and its sums.
- **Code lens** on each opener, `computed: <state> — Run`, runs the `computed.run` command. It renders the buffer as `run` would, under the trust store and the allowlist, and sends the result as a `workspace/applyEdit` rather than writing the file, so unsaved text is rendered as it stands and the change can be undone.

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

The repository's `action.yml` is a GitHub Action that installs a release, checked against its `SHA256SUMS`, and runs `check` with an annotation per region, or, with `command: suggest` on a pull request, posts what `run` would write as review suggestions. The Claude Code plugin registers `guard` as PreToolUse and PostToolUse hooks. Both are set out in [`docs/integrations.md`](../integrations.md).

## How is the crate laid out?

One package, `computed`, edition 2024, Rust 1.88 as the declared minimum. `src/lib.rs` holds every module; `src/main.rs` calls `computed::cli::main()`.

Dependencies: `clap`, `ignore`, `globset`, `sha2`, `tempfile`, `anyhow` (cli only), `similar`, `toml`, `serde`, `serde_json` (with `preserve_order`), `yaml-rust2`, `csv`, `wait-timeout`, `libc`, `tree-sitter` with the Rust, Python and Go grammars, `ureq` over rustls, `notify`, `lsp-server` and `lsp-types`, and `landlock` on Linux. No `regex`: the opener tokeniser is hand-written because of the quoting rules. Nothing links a system library beyond libc. Dev: `assert_cmd`, `tempfile`. No snapshot-testing crate.

The core, as milestone 1 built it:

- **`marker`.** Parses a file into `File { segments }`, each `Segment::Prose(String)` or `Segment::Region(Region)`. `Region` carries `line`, `indent`, `raw_opener`, `raw_closer`, `body` as the bytes sit, the closer's `in`/`out` sums, and the parsed `Opener { loader, flags, attrs, name, sink, lang, on_stale, max_lines }`. The grammar table names every loader's attributes, flags and default sink. `Opener::canonical()` gives the single-space form without suffix or indent. `serialise(&File) -> String` is the inverse. Parse errors are `ParseError { line, message }`, all tier 2.
- **`loader`.** `enum Loader`, one variant per loader, built from an `Opener`; `format_constant() -> u32`; the production `Loaders` adapter, `Production`, which expands recipes per file. Resolves every path against a per-file `Ctx { template, region_root, repo_root: Option<PathBuf> }` and makes the escape check there, the only place marker paths are resolved. Derives the three `COMPUTED_*` variables and runs every shell command through one core, which sets the pinned environment, applies any wrap and then the sandbox. Records the read set of every snapshot, which `cli` uses to settle. `LoadError::Hard(String)` is tier 2 for its region (escape, empty glob, a projection that finds nothing, unreadable input); `LoadError::Failed { stderr }` is tier 1 (exit status, timeout, bad UTF-8, failed normalisation); `LoadError::NotAllowed` is a disallowed url, tier 1.
- **`sink`.** `raw`, `fence` and `table`, pure: `Loaded` text in, body out. Normalisation, the `max-lines` cut and the parse-back check sit beside them.
- **`render`.** `file(parsed: &File, mode: Mode, trusted: bool, loaders: &mut dyn Loaders) -> Rendered`. Pure, no I/O. `file_where` takes a selection too, for `--only`. `Mode { Run { force }, DryRun { force }, Check, Clean { force, dry_run } }`. `Loaders` has two methods: `snapshot(&Region) -> Result<Option<Vec<u8>>, LoadError>`, where `None` is volatile, and `load(&Region) -> Result<Loaded, LoadError>`. `Rendered { Written { text, regions }, Unchanged { regions }, Refused { regions }, Error { line, message } }`. `render` owns the sums, the freshness cache, the states, the refuse rule and its per-file consequence, the trust test (`needs_trust`), the untrusted and disallowed skips, loader failure and hard errors keeping the body, `on-stale=warn`, the body's indentation and line endings, the whole-file parse-back check, and `clean`. Fresh regions are emitted from their raw lines. Per region it returns `RegionReport { line, name: Option<String>, loader, state, action, stderr: Option<String> }`.
- **`fs`.** `walk(root, WalkOpts { depth, all, dirs }) -> impl Iterator<Item = Entry>` with the `ignore` settings in exactly one place, used by discovery and by `tree`; `Ignores`, the same `.gitignore` rules matched path by path for `inputs=` expansion, with a test that a `**` expansion selects exactly the files the walk lists; `repo_root(path) -> Option<PathBuf>`, walking up for `.git` and canonicalising; `write(path, text)` through temp and rename, no-op when unchanged, writing through a symlink rather than replacing it; `replace(path, old, new)`, the same, only while the file still holds `old`.
- **`trust`.** The `trust.toml` store under `XDG_CONFIG_HOME`, store path injectable; `grant`, `revoke`, `is_trusted(root)`. Uses `fs::repo_root`.
- **`report`.** The stderr line per region, loader stderr indented beneath; the unified diff on stdout from old and new text via `similar`, only for `--dry-run`; the `--format json` document, written by hand.
- **`cli`.** clap definitions, discovery, per-file `Ctx` and trust resolution, the mapping from `Rendered` to a write and an exit tier, the passes that settle templates reading each other, and one dispatch arm per command. The only module using `anyhow`.

Loaders, sinks and what they share:

- **`project`.** The four projections: parse, canonical form, apply, and the byte range `adopt` writes into.
- **`index`**, **`toc`**, **`table`**. The pure text of the `index` and `toc` loaders and of the `table` sink.
- **`truncate`.** The `max-lines` cut and its note.
- **`symbol`**, **`git`**, **`remote`**, **`transcript`**. One loader each.
- **`config`.** `computed.toml`, its lookup, and recipe expansion.
- **`allow`.** The allowlist store and prefix matching.
- **`launch`**, **`sandbox`**. How a command is started, the `Wrap` that `doctor` and `trace` put around it, and the sandbox that goes on last.

Commands beyond the core, one module each: **`update`**, **`doctor`**, **`trace`**, **`affected`**, **`graph`**, **`why`**, **`merge`**, **`adopt`**, **`dupes`**, **`stats`**, **`guard`**, **`watch`**, **`lsp`**. **`survey`** is what the inspecting commands share: templates read and parsed as `run` reads them, path helpers, and the `git` runner.

Why a `Loaders` trait when the loader set is an enum? They are different things ([ADR 0008](../adr/0008-render-is-pure-behind-a-loaders-seam.md)). The loader set is closed, so it is an enum; ten variants did not change that. `Loaders` is a seam: it has two adapters, the production enum and a table-driven fake, which is the bar for a seam being real. `doctor`, the language server and `why` render through the same seam.

### Tests

- `render` through a fake `Loaders` against golden files under `tests/fixtures`: template in, text and reports out, covering the state table, refuse ordering, untrusted, loader failure and every mode. The golden files also fix the sum vectors.
- Unit tests beside every module: `marker` round trips and every grammar error, `sink` per variant and a table of byte sequences for normalisation, `fs::walk` over a tempdir with a `.gitignore`, `trust` and `allow` with an injected store path, each projection, each loader's text, the merge skeleton, the guard's rule, the trace log parsers.
- `cli`: `assert_cmd` for the three exit tiers and the pre-commit scenario.
- One end-to-end file per feature, over a temporary repository: `file_slice`, `value`, `index`, `toc`, `projected_inputs`, `table`, `symbol`, `history` (the `git` loader), `remote` (against a loopback server), `transcript`, `max_lines`, `on_stale`, `recipes`, `stats`, `affected` (with `graph`), `why`, `merge`, `adopt`, `dupes`, `doctor`, `trace` and `exec_sandbox` (with the real tracer and sandbox, skipped with a note where the machine has none), `guard` (fed sample hook JSON), `watch`, `lsp` (in process over an in-memory connection).

## What was milestone 1?

Milestone 1 is done. It is kept here as the record of what the foundation promised, and every acceptance line below still holds.

Milestone 1 was done when this repository dogfooded itself: a `CLAUDE.md` at the root with a tree region and an exec region, kept current by `computed run` in pre-commit and verified by `computed check` in CI. One `/implement` session, test-first at the seams above, built it in this order, each step green before the next: `marker`; `sink`; `render` with the fake `Loaders`; `fs`; `loader` with `tree` and `exec`; `trust`; `report` and `cli` with the five commands of the time; then the prototype modules were deleted and the package renamed to `computed`.

**Dogfood.** `CLAUDE.md` in this repository has two regions:

````markdown
## Layout

<!-- computed tree src=. depth=2 name=layout -->
<!-- /computed -->

## Decisions

<!-- computed index src=docs/adr/*.md name=adrs -->
<!-- /computed -->
````

The tree region exercises the walk and the listing snapshot. The decisions region began as an exec region over a script, which exercised `inputs=`, the content snapshot, and trust; the `index` loader replaced it once it existed, and the claude-code-plugin's `REFERENCE.md` carries the exec region now.

**Acceptance.** All of these, run from the repository root:

- `computed run` on the fresh `CLAUDE.md` writes both regions and exits 1; a second `computed run` writes nothing and exits 0; `computed check` exits 0.
- Add a file under `src/` and `computed check` exits 1 with the `layout` region `stale`; `computed run` writes it.
- Edit a line inside the `adrs` body and `computed run` exits 1 with `refused`, leaving the file untouched; `computed run --force` overwrites; `computed check` exits 0.
- `computed untrust`, then `computed run` reports an exec region `untrusted` and exits 1, still renders the tree region; `computed run --trust` renders both.
- `computed clean` then `computed check` exits 1 with both regions `unrendered`; `computed run` restores them and `computed check` exits 0.
- Remove the `out=` from one closer and `computed run` exits 2 with a parse error, writing nothing.
- `computed run --dry-run` after touching an input prints a unified diff to stdout, writes nothing, exits 1.
- `cargo test` passes and the `render` golden files contain the sum vectors.

**Not in milestone 1**, by decision: `watch`, copy layout, sinks beyond `raw` and `fence`, configuration files, colour, Windows. `file`, JSON reporting, `watch`, the `table` sink and `computed.toml` came after it.

## What is still open?

Each item waits on something dogfooding will show.

- **Formatters.** Whether sinks emit prettier and markdownlint range-ignore comments, so a formatter and a region stop undoing each other.
- **Reporting.** Colour.
- **Blockquotes.** A marker inside `>` is prose; regions do not nest in quote containers.
- **`symbol` languages.** TypeScript is left out: its grammar crate ships TypeScript and TSX together, which doubles the cost.
- **`watch` scope.** Any change under a watched root that `.gitignore` does not ignore triggers a full pass, even when no region reads it.
- **The Linux sandbox** has run on aarch64 with Landlock ABI 9 only. It wants a CI run on x86_64 and an older kernel.
- **A second dogfood repository**, if one is named.

Out of scope, and returning only if the destination is redrawn: copy layout, symlink layouts, file formats other than markdown, image sinks, nesting, regions that read other regions, Windows, Elixir.

## Decision record

| ADR | Decision |
|---|---|
| [0001](../adr/0001-rust-for-the-prototype.md) | Rust, one static binary. |
| [0002](../adr/0002-two-sum-closer.md) | Two sums in the closer. |
| [0003](../adr/0003-in-place-layout.md) | In-place is the only layout. |
| [0004](../adr/0004-region-root-is-the-template-directory.md) | Relative paths resolve against the template's directory. |
| [0005](../adr/0005-refuse-hand-edited-regions.md) | `run` refuses a hand-edited region. |
| [0006](../adr/0006-check-never-runs-a-loader.md) | `check` compares sums and never runs a loader. Amended by 0018. |
| [0007](../adr/0007-exec-trust-per-clone.md) | Exec trust is granted per clone, outside the working tree. |
| [0008](../adr/0008-render-is-pure-behind-a-loaders-seam.md) | Render is pure behind a `Loaders` seam. |
| [0009](../adr/0009-loader-text-is-normalised-and-exec-runs-pinned.md) | Loader text is normalised and exec runs in a pinned environment. |
| [0010](../adr/0010-sha-256-sums.md) | Sums are full SHA-256. |
| [0011](../adr/0011-gitignore-is-not-a-flag.md) | The tree loader honours `.gitignore` without a flag. |
| [0012](../adr/0012-wildcards-in-inputs-do-not-reach-ignored-paths.md) | Wildcards in `inputs=` do not reach ignored paths. |
| [0013](../adr/0013-a-region-the-tool-cannot-answer-skips-only-itself.md) | A region the tool cannot answer skips only itself. |
| [0014](../adr/0014-snapshots-ignore-sums-and-run-settles-across-files.md) | Snapshots ignore closer sums, and `run` settles templates that read each other. |
| [0015](../adr/0015-the-file-loader.md) | The `file` loader. |
| [0016](../adr/0016-a-projection-snapshots-only-the-part-it-reads.md) | A projection snapshots only the part it reads. |
| [0017](../adr/0017-trust-gates-running-repository-code-and-nothing-else.md) | Trust gates running repository code, and nothing else. |
| [0018](../adr/0018-the-git-snapshot-runs-git-under-check.md) | The `git` loader's snapshot runs `git` under `check`. |
| [0019](../adr/0019-remote-regions-are-pinned-and-allowlisted.md) | Remote regions are pinned by SHA-256 and fetch only under a per-machine allowlist. |
| [0020](../adr/0020-the-sandbox-enforces-inputs-and-does-not-replace-trust.md) | The sandbox enforces `inputs=` and does not replace trust. |
| [0021](../adr/0021-trace-reads-the-macos-sandbox-reports.md) | `trace` reads the macOS sandbox's own reports. |
| [0022](../adr/0022-recipes-in-computed-toml.md) | Recipes live in `computed.toml`, the first configuration file. |
| [0023](../adr/0023-on-stale-warn-softens-only-staleness.md) | `on-stale=warn` softens only staleness. |
| [0024](../adr/0024-the-merge-driver-leaves-doubly-rendered-regions-unrendered.md) | The merge driver leaves a region both sides re-rendered unrendered. |
| [0025](../adr/0025-the-guard-refuses-an-edit-before-it-lands.md) | The Claude Code guard refuses an edit to a region before it lands. |
