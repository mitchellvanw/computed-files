# computed reference

The marker grammar, the commands, and what every region state means. Setting the tool up is [`computed-setup/SKILL.md`](computed-setup/SKILL.md). Turning hand-written blocks into regions is [`discover-regions/SKILL.md`](discover-regions/SKILL.md).

## Markers

A region is an opener, a body the tool owns, and a closer.

~~~markdown
<!-- computed tree src=. depth=2 name=layout -->
<!-- /computed -->
~~~

`run` fills the body and rewrites the closer with two sums:

~~~markdown
<!-- /computed in=4b0267a3…c898 out=45ca178e…f559 -->
~~~

`in=` covers the canonical opener plus a snapshot of the region's inputs, so it answers "did the inputs move". `out=` covers the body the tool wrote, so it answers "did someone edit this by hand". Both are full SHA-256, and together they are the whole state. There is no cache directory and no sidecar.

Markers inside a fenced code block are prose, so an example like the ones above renders nothing.

**Code files.** A marker is written in the file's own comment, chosen by extension or name:

| Comment | Files |
|---|---|
| `<!-- -->` | Markdown, `.html`, `.xml`, `.svg` |
| `//` (also `///`, `//!`) | `.rs` `.go` `.ts` `.tsx` `.js` `.c` `.h` `.cpp` `.java` `.kt` `.swift` `.cs` `.dart` `.zig` `.proto` and kin |
| `#` | `.py` `.sh` `.rb` `.toml` `.yaml` `.yml` `.tf` `.nix` `.ini`, `Makefile`, `Dockerfile`, `.gitignore` |
| `--` | `.sql` `.lua` `.hs` |
| `/* */`, on one line | `.css` |

~~~rust
//! computed tree src=. depth=1 as=comment lang=text
//! /computed
~~~

In a `//` or `#` comment, a line is an opener only when `computed` is followed by a loader name, so `// computed values are cached` stays prose; `# computed tree is the loader` is a parse error. `as=comment` writes each line of the text behind the opener's leader, fenced first when `lang=` is set. Outside Markdown a loader that would fence its text writes it raw. `toc` works only in Markdown.

**Inline regions.** In Markdown and HTML, an opener and a closer on the same line hold one value of a sentence:

~~~markdown
The current release is <!-- computed value src=Cargo.toml key=package.version --><!-- /computed -->.
~~~

The text must be one line, and only `as=raw` applies; `max-lines=` is an error. Markers inside backticks are prose. `run` writes no `| do not edit` suffix inside a line. Reports name the place as `path:line:column`.

## Loaders

| Loader | Attributes | Renders | Default sink |
|---|---|---|---|
| `tree` | `src=.`, `depth=`, flags `all` and `dirs` | a directory listing, gitignore-aware | `fence` |
| `file` | `src=`, and at most one of `lines=A-B`, `section=TEXT`, `anchor=NAME` | a file, or that slice of it | `raw` |
| `value` | `src=`, `key=a.b.c` | one scalar of a `.toml`, `.json`, `.yaml` or `.yml` file | `raw` |
| `index` | `src="GLOB,GLOB"`, `title=h1\|filename` | `- [Title](path)` per matched file | `raw` |
| `toc` | `min=2`, `max=3` | this file's own headings as links | `raw` |
| `symbol` | `src=`, `item=`, `part=whole\|signature\|doc` | one item of a `.rs`, `.py` or `.go` file | `fence` in the file's language |
| `git` | one flag of `log`, `tags`, `contributors`; `src=.`, `n=10` | recent commits, tags or authors | `raw` |
| `remote` | `url=https://…`, `sha256=`, `timeout=30` | a document pinned by its SHA-256 | `raw` |
| `exec` | `cmd=`, `timeout=30`, exactly one of `inputs=` or `volatile`, flag `sandbox` | a command's stdout | `raw` |
| `transcript` | `steps="A ;; B"`, exactly one of `inputs=` or `volatile`, `timeout=`, `workdir=tmp\|copy`, flag `sandbox` | each step as `$ step` and its output | `fence`, `lang=console` |
| `use` | `recipe=NAME` and nothing else of the loader's | the opener named in `computed.toml` | the recipe's |

Every region also takes:

- `name=` for stable reports;
- `as=raw|fence|table|comment` to pick the sink; `table` reads CSV, `delim=tab` or `from=jsonl` and writes a Markdown table, and `comment` writes comment lines in a code file;
- `lang=` for the fence language;
- `max-lines=N` to cut the text to N lines and a `… K more lines` note;
- `on-stale=warn` to let `check` report the region stale without failing.

The tool writes `| do not edit; run computed` after the attributes; nothing else may follow a `|`. An indented opener, such as one inside a list item, gets its body indented to match. To show marker examples in a region, fence them: `file src=example.md as=fence lang=markdown`.

**Slices.** `lines=`, `section=` and `anchor=` on `file`, and `key=` on `value`, snapshot only that part of the file, so an edit elsewhere leaves the region fresh. `section=` matches a heading's text exactly, at any level, and runs to the next heading of the same or higher level. `anchor=NAME` takes the lines between `ANCHOR: NAME` and `ANCHOR_END: NAME`. A slice that finds nothing is an `error`.

**`inputs=`** is a comma-separated list of globs, with no empty entries, and it is what makes an exec or transcript region checkable: `check` re-snapshots those paths and compares sums without running the command. A literal path may take a slice after `#`: `inputs="Cargo.toml#key=package.version,src/cli.rs#lines=40-90,docs/*.md"`. `volatile` declares there is nothing worth snapshotting, so the region re-renders on every `run` and `check` always passes it. `sandbox` runs the command where it can read only its inputs and the system's programs and reach no network, so an undeclared read fails the region.

**Recipes.** A `computed.toml` in the template's directory or one above it, up to the repository root, holds `[recipe.NAME]` tables: `loader = "index"` and the loader's attributes, flags as `true`. `use recipe=NAME` stands for that opener; its sums are those of the opener it stands for, and editing the recipe makes its regions stale.

Relative paths in a marker resolve against the directory of the file that holds the marker, and an exec command runs there, not in the repository root and not in the shell's working directory. An exec command gets `COMPUTED_ROOT`, `COMPUTED_FILE` (absolute) and `COMPUTED_REGION` in its environment and runs under `LC_ALL=C`, `TZ=UTC` and an empty `LANGUAGE`, so it gives the same bytes on a laptop and in CI.

**Trust.** `exec` and `transcript` regions run only in a clone `computed trust` has granted, recorded in `~/.config/computed/trust.toml` and never in the working tree. Cloning a repository therefore executes nothing in it. Every other loader runs nothing from the repository and needs no grant. `remote` fetches only under a url prefix this machine allowed with `computed allow`; `computed update` fetches and writes the `sha256=` pin.

## Commands

<!-- computed exec cmd=../../scripts/cli-commands.sh inputs=../../src/cli.rs,../../scripts/cli-commands.sh name=commands as=fence | do not edit; run computed -->
```
computed run      [paths] [--force] [--dry-run] [--trust] [--only NAME] [--allow PREFIX]
computed check    [paths] [--only NAME]
computed clean    [paths] [--force] [--dry-run] [--only NAME]
computed update   [paths] [--dry-run] [--only NAME] [--allow PREFIX]
computed allow    [prefix]
computed disallow <prefix>
computed trust    [path]
computed untrust  [path]
computed stats    [paths]
computed guard    [file] [--proposed PATH] [--hook HOOK]
computed watch    [paths] [--trust] [--allow PREFIX]
computed lsp
computed affected <paths>
computed graph    [paths]
computed why      <file> [--only NAME] [--line N]
computed merge    [base] [ours] [theirs] [path] [--install]
computed adopt    <file> [--only NAME] [--dry-run]
computed dupes    [paths] [--min-lines N]
computed doctor   [paths] [--trust] [--allow PREFIX] [--only NAME]
computed trace    [paths] [--trust] [--only NAME] [--write]
```
<!-- /computed in=cf4759cba40877f3b9a6f31e76fc991db8917d8e16d5385dc346f46f8084fca9 out=bc0b59dc0cb69dca6d6c9db06e24b96bb529455e921031023662129eea9f69fc -->

With no paths, the current directory is walked with the tree loader's ignore settings, dot-directories such as `.claude/` included, and every `.md` and `.markdown` file is read. A `[discover]` table in the repository root's `computed.toml`, `extensions = ["rs"]` and `names = ["Makefile"]`, adds code files to the walk. An explicit file is read whatever its extension, in the comment its name selects. `run --dry-run` prints the diff `run` would write and writes nothing. `--only NAME` narrows a command to the regions with that name. `--format json` prints one JSON document on stdout instead of the report.

When one template's `inputs=` include another, `run` settles both in one invocation.

Beyond `run`, `check` and `clean`, reach for:

- `affected PATH` before changing a file, to see which regions it feeds; `why FILE` to see what moved since a stale region was rendered.
- `trace` to find what an exec command reads that `inputs=` misses, `trace --write` to fix the opener; `doctor` to find a region whose output changes between runs or moved without its inputs.
- `dupes` to find blocks copied between Markdown files, each with the `file` region that would replace the copy.
- `adopt FILE` to write a hand edit inside a `file` region back into its source.
- `stats` for how much of each file is computed; `graph` for what every region reads.

## States

One line per region goes to stderr, as `path:line name loader state`. Fresh regions print only under `-v`.

| State | Meaning | `run` | `check` |
|---|---|---|---|
| `fresh` | Both sums match. | leaves it alone | passes |
| `stale` | Inputs or the opener changed. | re-renders | exit 1 |
| `edited` | The body was changed by hand. | refuses the file, exit 1 | exit 1 |
| `volatile` | Declares no inputs. | re-renders every time | passes |
| `unrendered` | The closer carries no sums. | renders | exit 1 |
| `error` | The tool could not answer for this region. | skips it, renders the rest, exit 2 | exit 2 |

A stale region whose opener says `on-stale=warn` is reported `stale warn` by `check`, which still exits 0.

| Exit | Meaning |
|---|---|
| 0 | Everything is fresh. |
| 1 | The content said no: drift under `check`; a write, a refused file, a loader failure, or an untrusted or disallowed region under `run`. |
| 2 | The tool could not answer: usage error, marker parse error, a path escaping the repository, `inputs=` or a slice matching nothing, a file edited while `run` computed it. |

## Acting on a state

- **`stale`**. Run `computed run`.
- **`edited`**. The body no longer matches `out=`. `run` refuses the whole file, leaves every region in it untouched and exits 1. `computed run --dry-run` shows the diff `--force` would apply; keep the hand-written change somewhere if it was wanted, then `computed run --force` hands the body back to the tool.
- **`untrusted`**. An exec or transcript region in a clone with no grant. Run `computed trust`, or pass `run --trust` for a single invocation.
- **`disallowed`**. A remote region whose url is not on this machine's allowlist. `computed allow <prefix>` allows it; the message names the prefix. Pass `--allow <prefix>` for a single invocation.
- **A pin mismatch or a missing pin**. A remote document changed, or was never pinned. `computed update` fetches it and writes the new `sha256=`; review that one-line diff, then `computed run`.
- **A loader failure**. The command's stderr prints under the region's line, the last good body and sums stay put, and the run exits 1. Repair the input and run again.
- **`error`**. The message beneath names the cause: a glob or slice that matches nothing, a path outside the repository, a missing `computed.toml` recipe, a shallow clone under a `git` region. Fix the opener or the input it names.
