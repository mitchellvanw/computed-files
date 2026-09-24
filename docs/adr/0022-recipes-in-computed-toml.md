---
status: accepted
---

# Recipes live in `computed.toml`, the first configuration file

A repository may name openers in a `computed.toml`, one `[recipe.NAME]` table each, and a region writes `use recipe=NAME` for the opener the recipe stands for:

```toml
[recipe.adrs]
loader = "index"
src = "docs/adr/*.md"
```

The file holds `[recipe.*]` tables and nothing else. A recipe sets a loader, its attributes and flags, and any of the common attributes `as`, `lang`, `on-stale` and `max-lines`. A `use` opener may add common attributes, which override the recipe's, and may not add a loader attribute or flag. The file is found by walking up from the template's directory to the repository root, and the nearest one wins. The expansion is the region's opener for every purpose: its paths resolve against the using template's region root, its canonical form goes into the input sum, and an exec recipe needs trust as exec does.

This resolves the spec's open question, "Configuration: a `computed.toml`, or conventions only".

## Context

The spec deferred configuration because trust was already settled outside the repository ([ADR 0007](0007-exec-trust-per-clone.md)), so a configuration file would only have served discovery. Two needs came up in use. The same long opener appears in several files, such as an exec command in `CLAUDE.md`, `AGENTS.md` and `README.md`, and changing it means editing every copy. And an opener with a quoted `cmd=` and a long `inputs=` is hard to read in the middle of prose.

The constraints were already fixed. A file in the repository cannot grant trust. Rendering must not depend on the machine. And a template must keep meaning what it says: a reader of a `use` opener has to be able to find what it stands for.

## Considered Options

- **Conventions only:** long commands go in `scripts/`. No new file. Rejected as the whole answer: the script helps with `cmd=`, but `inputs=` and the attributes are still copied into every opener.
- **A configuration file in the user's config directory.** Rejected: rendering would then depend on the machine.
- **One `computed.toml` at the repository root only.** Simpler lookup. Rejected: a vendored package or a monorepo member with its own docs wants its own recipes. Nearest-wins is how `.gitignore` and `.editorconfig` already behave. The walk stops at the repository root, so a file outside the repository never changes how a template renders.
- **Loader attributes overridable at the use site.** More flexible. Rejected: a recipe would then mean different things in different files. Common attributes are presentation and a name, so they may differ per use.
- **Sums over the recipe's name.** Rejected: renaming a recipe would make every region stale for no change. The sums are over the expansion, so a `use` region has the same sums as the equivalent inline opener. Switching between the two forms, or renaming a recipe that keeps its definition, is not a change. Editing the recipe is.
- **Recipes defined in a Markdown file,** as regions other regions refer to. Rejected: that is a region reading another region, which the spec keeps out of scope.
- **`[recipe.*]` tables in `computed.toml`, nearest file up to the repository root.** Chosen. TOML was already a dependency.

## Consequences

- `computed.toml` is the first file that changes how a template renders without the template naming it. `graph` labels a region `exec via recipe X`, `affected computed.toml` lists every `use` region, and the editor hover shows the opener a region expanded from.
- The file is in the read set, so `run` settles after an edit to it, and `watch` notices one.
- An error anywhere in `computed.toml` makes every `use` region that reads it an `error`, and only those ([ADR 0013](0013-a-region-the-tool-cannot-answer-skips-only-itself.md)). A template with no `use` region never reads the file. Messages name the file and the `[recipe.NAME]`.
- A recipe cannot use another recipe. Any other top-level key is an error, so the file cannot grow a second discovery or trust mechanism unnoticed.
- Every code path that renders or snapshots has to expand recipes first, through `Production::for_file`. A command that skips it reports every `use` region as an error. `why` expands a historic region with that commit's `computed.toml`.
- A volatile recipe behaves like any volatile region: a change to it is not visible to `check`.
