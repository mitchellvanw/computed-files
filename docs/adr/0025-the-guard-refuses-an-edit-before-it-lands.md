---
status: accepted
---

# The Claude Code guard refuses an edit to a region before it lands

The Claude Code plugin registers two hooks on `Edit`, `MultiEdit` and `Write`. Before the tool runs, `computed guard --hook pre` applies the proposed edit to the file's current text in memory and compares the two. If the edit changes a region's body or closer, the hook answers `permissionDecision: deny`, with a reason that names the region, its opener, and what to edit instead: the source it is computed from, or the opener. An edit that moves a region intact is allowed. So is one that removes or rewrites a region together with its opener. After the tool runs, `computed guard --hook post` checks the edited file and hands any drift back to the agent as context. Both hooks always exit 0, and if `computed` is not installed they allow everything.

## Context

`run` refuses a hand-edited region ([ADR 0005](0005-refuse-hand-edited-regions.md)), and that ADR's argument is that a failed pre-commit hook is the one signal that reaches an agent. It reaches it late. The agent edits a region body, keeps working for many turns, and learns at commit time that the edit is refused. By then later changes rest on it, and someone has to run `run --dry-run`, save the edit somewhere, and `run --force`.

In Claude Code, the agent's file edits go through tool calls, and a PreToolUse hook sees each one before it is applied. The cheapest moment to say "this region is computed from X, edit X" is before the edit lands.

## Considered Options

- **Commit time only.** No hook to maintain. Rejected: for an agent it is the most expensive moment. The edit may be many turns old and the context that produced it gone.
- **Let the edit land, then report or revert it** in PostToolUse. Rejected: reverting means the tool writing behind the agent's back, and later edits would be matched against text that has vanished. A deny before the write costs nothing, and the reason tells the agent where the real source is.
- **Deny every edit to a file with regions.** A trivial rule. Rejected: prose is the author's, and `CLAUDE.md` is mostly prose.
- **Answer `ask` instead of `deny`.** The person decides. Rejected: in an unattended session it stops everything for a mistake the agent can fix itself once it has the reason. A person who does want to change a body has `run --force`.
- **Deny body and closer changes before the edit, report drift after it.** Chosen.

## Consequences

- Only `Edit`, `MultiEdit` and `Write` are guarded. An edit made through the shell, such as `sed -i`, reaches the file and is caught by `run` at commit time as before.
- The post-edit check covers the edited file only, not the templates that read it.
- Deleting an opener, body and closer together is allowed, and so is writing a new region. Rewriting a region whole with its sums stripped is allowed too; the region is then unrendered and `run` renders over it.
- If the Edit tool matched `old_string` only after normalising quotes, the guard cannot find the literal text and allows the edit.
- The same judgement is available outside Claude Code as `computed guard FILE --proposed PATH`: exit 0 when allowed, 1 when a region is touched or the markers break, 2 on a usage error. It says 1 for a marker break, where a parse error elsewhere is 2, because the question asked was "may this edit land", and the answer is no.
- Without the plugin, the same two hooks go in `.claude/settings.json` as `computed guard --hook pre` and `computed guard --hook post`.
