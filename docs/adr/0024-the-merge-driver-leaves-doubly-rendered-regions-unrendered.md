---
status: accepted
---

# The merge driver merges by structure and leaves a region both sides re-rendered unrendered

`computed merge BASE OURS THEIRS [PATH]` is a git merge driver for templates. It replaces every region's body and closer, in all three versions, with one placeholder line keyed by the region's `name=` or its canonical opener. It merges the skeletons with `git merge-file`, so prose and opener conflicts stay ordinary conflict markers. Then it puts each region back. If the two sides agree, or only one changed the region, that side's body and closer are kept. If both changed it differently, the region gets ours' body under a closer with no sums, so it is `unrendered` and the next `run` renders it from the merged inputs. A version that does not parse falls back to a plain `git merge-file`.

`computed merge --install` appends `*.md merge=computed` and `*.markdown merge=computed` to the repository's `.gitattributes` and sets `merge.computed.driver` in this clone's git config.

## Context

Two branches that each add a file under `src/` each re-render the layout tree. The bodies differ and so do the sums in both closers, so every merge of the two conflicts inside a region neither side should resolve by hand. The right body is whatever the merged inputs render to, and only a `run` after the merge knows that. Integrating the features this ADR belongs to hit that conflict in `CLAUDE.md` and `REFERENCE.md` on every branch.

Resolving it by hand is worse than tedious. Picking one side and editing it leaves a body that no longer matches `out=`, so the region is `edited`, `run` refuses the file, and the person has to reach for `--force`.

## Considered Options

- **Classify the conflict hunks of an ordinary merge,** and resolve the ones that fall inside a region. No change to how git merges. Rejected: `git merge-file` joins conflicts separated only by lines with no letters or digits, so a closer conflict and a prose conflict one blank line apart come back as one hunk.
- **Render inside the driver.** The merge would come out fresh. Rejected: the driver sees three blobs of one file while other files may still be mid-merge, so the merged inputs are not on disk yet. An exec region would need trust, and a merge that runs commands is a surprise.
- **Take ours' body and closer when both changed.** Rejected: ours' sums describe ours' inputs, and the merge is neither side. `check` would probably call it stale, but the file would still be claiming a render that never happened.
- **Leave doubly changed regions `unrendered`.** Chosen. An unrendered closer claims nothing, `check` fails on it until someone runs `run`, and `run` renders it whatever the body holds, so the refuse rule never gets involved.

## Consequences

- A merge that only re-rendered the same region on both sides merges cleanly. `check` exits 1 afterwards, and `run` settles it.
- The `.gitattributes` lines are committed, but the driver itself is configured per clone, like trust. In a clone that never ran `--install`, git has no driver by that name and uses its ordinary text merge.
- The driver calls whatever `computed` is on `PATH`.
- Only `.md` and `.markdown` files are routed. A template with another extension, such as `docs/index.html`, still merges line by line.
- A version containing the placeholder string falls back to a plain merge. So does one that is not UTF-8.
