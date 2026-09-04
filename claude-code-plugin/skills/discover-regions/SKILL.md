---
name: discover-regions
description: Find hand-written blocks in a repository's Markdown that go stale on their own, such as a file tree, a command's output or a generated index, and turn them into computed regions. Use when adding a computed region to a file, when asked what is worth computing in a project, or after `computed-setup` has put the tool in place.
---

# discover-regions

A `computed` region is a span of Markdown the tool owns and rewrites when the inputs it was computed from move. This skill finds the spans worth handing over and writes the markers.

It assumes `computed --version` answers. When it does not, `computed-setup` installs and wires the tool first.

Done when every region chosen renders, `computed run` writes nothing on a second pass, and `computed check` exits 0.

The marker grammar, the loaders and every attribute live in [`REFERENCE.md`](../REFERENCE.md). Read it before writing a marker that is not one of the templates in step 3.

## 1. Discover

Find the Markdown, then read it. `grep -rl '<!-- computed' --include='*.md' .` names the files that already have regions. Leave those regions alone and look at the prose around them.

A block is worth computing when the repository already knows its content and a person has copied it out by hand. Judge each candidate by what would make it wrong.

| Block | Goes wrong when | Loader |
|---|---|---|
| A file tree or directory listing | Any file is added or moved | `tree` |
| A command's output, pasted | The command's output changes | `exec` with `inputs=` |
| An index built from files, such as ADRs or migrations | A file is added or its title changes | `exec` with `inputs=` |
| A table of contents | A heading changes | `exec` with `inputs=` |
| A version, a count, a date stamp | Almost at once | `exec` |
| Anything a person decided | It does not. A person changes it on purpose | leave it alone |

That last row is the one to get right. Prose, rationale, a hand-picked example and a table of judgements are not stale, they are edited. Computing them takes the decision away from the person making it.

`CLAUDE.md` and `README.md` are where these blocks collect, because both are read constantly and written once.

## 2. Ask

One `AskUserQuestion` call. Name each candidate by file and by what it holds, so the choice can be made without opening anything.

**Which blocks should computed own?** Use `multiSelect: true` and at most four candidates, the ones that go wrong soonest first. Say in each description what the region would compute and what would make it stale.

When more than four are worth offering, take the first four and say in the report that others are waiting.

Ask about the loader only where the table above leaves it open. A tree is a tree.

## 3. Write

1. **Trust the clone** when any chosen region uses `exec`. Run `computed trust`. Until a grant exists, `run` skips exec regions, keeps their bodies and exits 1. A `check`-only CI pipeline still needs nothing.
2. **Replace each block with a marker pair.** The opener goes where the block started, the body is empty, the closer carries no sums. Delete the hand-written content. `run` writes it back computed.

   ~~~markdown
   <!-- computed tree src=. depth=2 name=layout -->
   <!-- /computed -->
   ~~~

   ~~~markdown
   <!-- computed exec cmd="<command>" inputs=<glob>,<glob> name=<name> as=fence -->
   <!-- /computed -->
   ~~~

   An exec region takes exactly one of `inputs=` or `volatile`. Point `inputs=` at the files whose change should make the region stale. That is what lets `check` do its job without running the command. Reach for `volatile` only when nothing on disk determines the output.

   A command long enough to need quoting belongs in a script under `scripts/`, named in `cmd=` and listed in `inputs=` alongside what it reads, so editing the script makes the region stale too.
3. **Render and verify.** `computed run` exits 1 the first time because it wrote a file. Run it again and it writes nothing and exits 0. Then `computed check` exits 0. That is the bar. Do not stop before it.
4. **Read what came back.** A region that renders empty or wrong is a marker to fix, not output to accept. Compare it against the block that was there before.

## 4. Report

Name each region written, what it computes, and what makes it stale. Name any candidate you passed over and why, so nobody has to decide it again.
