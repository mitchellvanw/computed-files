---
status: accepted
---

# Wildcards in `inputs=` do not reach ignored paths

Inside a git repository, a wildcard component of an `inputs=` glob does not select a path the repository's `.gitignore` files ignore, and never selects `.git`. A literal component always reaches what it names, ignored or not. `**/*.md` skips `target/` and `.claude/worktrees/`; `target/coverage.json` and `.git/HEAD` still select those files. The ignore settings are the `tree` loader's, pinned in the spec, and outside a repository no rules apply.

The rule is checked per entry, as expansion reaches it: an entry reached through a wildcard part is dropped when it is itself ignored, while an entry under a directory a literal part named is checked only against its own name. So `target/*.json` selects the JSON files in an ignored `target/`, and `**/*.json` does not.

## Context

The spec says "Ignore rules do not filter declared inputs." Until dax-php#749, every expansion walked the glob's literal prefix whole, so `third-engine-design*.md` read the repository root, `.git` and every ignored build directory, and failed when a cargo build deleted a file mid-walk. Pruning expansion to directories a glob can match, and skipping a path that vanishes, fixed that failure without touching this rule. A glob with `**` still reaches every directory, and that exposes two costs the rule has always carried:

- **The input sum depends on the machine.** On a clone with worktrees under `.claude/worktrees/`, `**/*.md` hashes each worktree's copies of the Markdown; CI's clone has none. The region the pre-commit hook rendered is stale in CI, and a local `run` rewrites it whenever a worktree comes or goes. The `tree` loader turned per-clone and per-user exclude files off for exactly this reason: they "would make the snapshot differ between machines". Ignored files do the same, from inside the working tree.
- **`.git` is hashed.** `**` matches the object store and the index, so every commit changes the input sum of every `**` region, and `check` reads the whole object store to say so.

Neither cost comes from a glob someone meant. Nobody writes `**/*.md` wanting build output or git's internals in it.

## Considered Options

- **Keep the rule: ignore rules never filter inputs.** One sentence, no ignore machinery in the exec loader, and a snapshot that holds exactly what the glob says. Rejected because the glob's literal meaning changes between machines: the same `**` region is fresh on one clone and stale on another, which is the reproducibility failure the spec otherwise guards against, and `.git` makes it stale on every commit.
- **Apply the ignore rules to every match, as `tree` does.** One rule for both loaders, and the simplest to state. Rejected because an ignored file is sometimes the real input: a coverage report, a generated schema or a build manifest named in `inputs=` so the region reruns when it changes. Under this option `target/coverage.json` matches nothing, a tier-2 error, and there is no way to name it.
- **Exclude `.git` only.** Fixes the commit-by-commit drift with one hard-coded name. Rejected as the whole answer because it leaves the machine-dependent sum in place, but it is part of the chosen rule.
- **Filter ignored paths, with a flag to include them.** Mirrors `tree`'s `all`. Rejected because the literal-component rule already gives that escape without growing the closed flag set: naming the ignored directory is the opt-in.
- **Wildcards skip ignored paths and `.git`; literal components reach anything.** Chosen. It matches how ripgrep treats an ignored path passed by name, and it makes every selection intentional: whatever a snapshot holds, either nothing ignores it or the glob named it.

## Consequences

- The spec's `exec` section replaces "Ignore rules do not filter declared inputs" with this rule.
- A `**` region whose snapshot held ignored files or `.git` gets a different input sum once, and `check` reports it stale until it is run again. No format constant changes: the rendering of the same inputs is unchanged, and the inputs are what moved.
- An edit to `.gitignore` can move the input sum of a wildcard region, as it can a `tree` listing ([ADR 0011](0011-gitignore-is-not-a-flag.md)). An edit that changes nothing selected is not drift.
- Expansion builds gitignore matchers for the directories it enters, with the `ignore` crate's `Gitignore`, already a dependency. The walk's `WalkBuilder` cannot answer for one path at a time, so this is a second reading of the same settings: `fs::Ignores` sits beside `fs::walk`, and a test pins that a `**` expansion selects exactly the files the `tree` walk lists, negations and nested files included.
- "A glob that matches nothing" can now mean "matches only ignored files". The error says `matches nothing that is not ignored` when that is why, so the rule does not read as a bug.
- The rule is harder to state than either extreme: "wildcards obey `.gitignore`, names do not" is one sentence, but `target/*.json` versus `**/*.json` has to be shown, not just stated.
