---
name: discover-regions
description: Find hand-written blocks in a repository's Markdown and code comments that go stale on their own, such as a file tree, a version in a sentence, a command's output or a generated index, and turn them into computed regions. Use when adding a computed region to a file, when asked what is worth computing in a project, or after `computed-setup` has put the tool in place.
---

# discover-regions

A `computed` region is a span of a file the tool owns and rewrites when the inputs it was computed from move: lines of Markdown, a value inside a sentence, or comment lines in a code file. This skill finds the spans worth handing over and writes the markers.

It assumes `computed --version` answers. When it does not, `computed-setup` installs and wires the tool first.

Done when every region chosen renders, `computed run` writes nothing on a second pass, and `computed check` exits 0.

The marker grammar, the loaders and every attribute live in [`REFERENCE.md`](../REFERENCE.md). Read it before writing a marker that is not one of the templates in step 3.

## 1. Discover

Find the Markdown, then read it, `.claude/` and `.github/` included. `grep -rl '<!-- computed' --include='*.md' .` names the files that already have regions. Then skim the doc comments of the code: a crate's `//!` module list or a Makefile's help text copies what the repository knows too. Leave those regions alone and look at the prose around them. `computed dupes` lists blocks copied between Markdown files, each with the `file` region that would replace the copy.

A block is worth computing when the repository already knows its content and a person has copied it out by hand. Judge each candidate by what would make it wrong, and pick the loader from this table. Prefer a native loader over `exec` wherever one fits: it needs no trust, so a fresh clone renders it, and its snapshot is exactly the part it reads, so it goes stale only when that part changes.

| Block | Goes wrong when | Loader |
|---|---|---|
| A file tree or directory listing | A file is added or moved | `tree` |
| A version, or another field of a manifest | That field changes | `value src=Cargo.toml key=package.version`, inline when it sits in a sentence |
| An index of files, such as ADRs or migrations | A file is added or its title changes | `index src="docs/adr/*.md"` |
| A table of contents | A heading changes | `toc` |
| A section copied between files, such as `CLAUDE.md` and `AGENTS.md` | One copy is edited and the other is not | `file src=… section="…"` |
| Lines of a source or config file shown as an example | Those lines change | `file src=… lines=A-B as=fence lang=…`, or `anchor=` |
| A function's signature, a type, or a doc comment | The code changes | `symbol src=… item=… part=signature` |
| Recent commits, releases or contributors | A commit or a tag lands | `git log`, `git tags`, `git contributors` |
| A document shared from another repository | Its owner changes it | `remote url=…`, pinned by `computed update` |
| A command's output, pasted | The command's output changes | `exec` with `inputs=` |
| A list in a code comment, such as the modules a crate doc names | A file is added or renamed | `tree` or `index` with `as=comment`, in the file's own comment |
| A terminal session showing how to use a tool | The tool's behaviour changes | `transcript` with `inputs=` |
| Anything a person decided | It does not. A person changes it on purpose | leave it alone |

That last row is the one to get right. Prose, rationale, a hand-picked example and a table of judgements are not stale, they are edited. Computing them takes the decision away from the person making it.

`CLAUDE.md` and `README.md` are where these blocks collect, because both are read constantly and written once.

## 2. Ask

One `AskUserQuestion` call. Name each candidate by file and by what it holds, so the choice can be made without opening anything.

**Which blocks should computed own?** Use `multiSelect: true` and at most four candidates, the ones that go wrong soonest first. Say in each description what the region would compute and what would make it stale.

When more than four are worth offering, take the first four and say in the report that others are waiting.

Ask about the loader only where the table above leaves it open. A tree is a tree.

## 3. Write

1. **Trust the clone** when any chosen region uses `exec` or `transcript`. Run `computed trust`. Until a grant exists, `run` skips those regions, keeps their bodies and exits 1. A `check`-only CI pipeline still needs nothing, and no other loader needs a grant.
2. **Replace each block with a marker pair.** The opener goes where the block started, the body is empty, the closer carries no sums. Delete the hand-written content. `run` writes it back computed.

   ~~~markdown
   <!-- computed tree src=. depth=2 name=layout -->
   <!-- /computed -->
   ~~~

   ~~~markdown
   <!-- computed file src=README.md section="Install" name=install -->
   <!-- /computed -->
   ~~~

   ~~~markdown
   <!-- computed exec cmd="<command>" inputs=<glob>,<glob> name=<name> as=fence -->
   <!-- /computed -->
   ~~~

   A value in a sentence becomes an inline region: both markers on its line, the value between them deleted. Its text must be one line.

   ~~~markdown
   Requires computed <!-- computed value src=Cargo.toml key=package.version name=version --><!-- /computed --> or later.
   ~~~

   In a code file, the markers go in the file's own comment, and `as=comment` keeps the body a comment:

   ~~~rust
   //! computed tree src=. depth=1 as=comment lang=text
   //! /computed
   ~~~

   A bare `computed run` reads only Markdown. For a code file to be found by the hook and CI, add its extension to `[discover]` in `computed.toml` at the repository root: `extensions = ["rs"]`, or `names = ["Makefile"]`. The merge driver's `--install` routes only Markdown; add a line such as `*.rs merge=computed` to `.gitattributes` when the merge driver is in use.

   An exec region takes exactly one of `inputs=` or `volatile`. Point `inputs=` at the files whose change should make the region stale. That is what lets `check` do its job without running the command. When the command reads only part of a file, name the part: `inputs="Cargo.toml#key=package.version"`. Reach for `volatile` only when nothing on disk determines the output.

   A command long enough to need quoting belongs in a script under `scripts/`, named in `cmd=` and listed in `inputs=` alongside what it reads, so editing the script makes the region stale too.
3. **Shape what needs it.**
   - Output that can run long gets `max-lines=N`, which keeps the first N lines and a note of how many were cut.
   - A region over inputs that move on every commit, such as `git log` of the whole repository, can take `on-stale=warn` when the user does not want CI to block on it. `check` still reports it stale, and exits 0.
   - An opener that would be repeated in more than one file becomes a recipe: a `[recipe.NAME]` table in `computed.toml` at the repository root, and `use recipe=NAME` in each file.
   - A `remote` region needs its prefix allowed on this machine (`computed allow <prefix>`) and a pin (`computed update`) before `run` renders it.
4. **Render and verify.** `computed run` exits 1 the first time because it wrote a file. Run it again and it writes nothing and exits 0. Then `computed check` exits 0. That is the bar. Do not stop before it.
5. **Check every exec region's `inputs=`.** `computed trace` runs each command under a file tracer and names the files it read that `inputs=` does not declare. An undeclared read is a region `check` will call fresh when it is not. `computed trace --write` puts its suggestion in the opener; run `computed run` again after it. When `trace` exits 2 because this machine has no tracer, say so in the report.
6. **Read what came back.** A region that renders empty or wrong is a marker to fix, not output to accept. Compare it against the block that was there before.

## 4. Report

Name each region written, what it computes, and what makes it stale. Name any candidate you passed over and why, so nobody has to decide it again.
