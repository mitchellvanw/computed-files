---
status: accepted
---

# Relative paths resolve against the template's directory

Every relative path in a marker, the tree loader's `src=`, exec's `inputs=` globs, and the working directory an exec command runs in, resolves against the directory of the template file, not the repository root or the directory `computed` was invoked from. mdsh does the same with its `--work-dir` default. A region then reads identically whether the file is `CLAUDE.md` at the root or `docs/guide.md`, moving a file moves its regions with it, and a region in a file that is not inside a git repository still has a meaning. The repository root, when there is one, reaches the command as `COMPUTED_ROOT`.

## Considered Options

- **Repository root.** Rejected: every marker in a nested file must spell its own path back to itself, a file outside a repository has no root, and moving a file silently breaks its regions.
- **Invocation directory.** Rejected: the same file renders differently from a pre-commit hook, CI and a shell, which defeats `check`.
- **The template's directory.** Chosen. Cost: an exec command that wants the repository root must say `cd "$COMPUTED_ROOT"` or use the variable, and a marker cannot be copied between files at different depths without editing its paths.

## Consequences

- Once files carry markers, changing the base moves every region's inputs. This is why the choice is recorded rather than left as a default.
- `tree src=.` lists the template's own directory, which is the common case for a `CLAUDE.md` layout region.
