---
status: accepted
---

# The tree loader honours `.gitignore` without a flag

Inside a git repository the `tree` loader always applies the repository's `.gitignore` files, root and nested, with the `ignore` crate settings the spec fixes. There is no `gitignore` flag: an opener that writes one is an unknown-flag parse error. Outside a repository no ignore rules apply.

The ignore state is covered by the snapshot without being read into it. The snapshot is the listing, so an edit to `.gitignore` that changes which paths appear changes the input sum, and `check` reports the region stale like any other input change. An edit that leaves the listing as it was, a comment or a pattern that matches nothing, is not drift.

## Considered Options

- **An opt-in `gitignore` flag, default off.** Rejected. Every real use of a tree region in a repository wants the ignore rules; a default that lists `target/` is a default nobody chooses, and the dogfood openers in this repository would all grow the flag. It also contradicts the spec's tree section, which has said since v0 that the loader is gitignore-aware.
- **An accepted flag that changes nothing.** Rejected. A flag whose presence and absence render the same bytes is a lie in the grammar, and the closed flag set is the one thing that lets a reader trust an opener.
- **Snapshot the `.gitignore` bytes as well as the listing.** Rejected. A comment edit would drift every tree region in the repository for no change in the rendered body, and the spec fixes the snapshot as the listing alone.
- **Always on inside a repository, no flag.** Chosen. It is what the code has done since the walk was written; this record pins it and adds the tests that were missing: one per pattern class, and the drift on a `.gitignore` edit end to end.

## Consequences

- The `tree` grammar is unchanged: `src=`, `depth=`, `all`, `dirs`. Adding a flag later is cheap; this decision removes nothing.
- The issue that asked for this described the loader as accepting and ignoring a `gitignore` attribute, and cited a spec example carrying it. Both were stale: the parser rejects the flag, and no spec example uses it. The `.gitignore` and ignored file the issue asks the demo repository to gain were already in the CLI fixture; the assertion that the tree excludes them was not, and is now.
