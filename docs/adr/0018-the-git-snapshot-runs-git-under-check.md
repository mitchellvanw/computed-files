---
status: accepted
---

# The `git` loader's snapshot runs `git` under `check`

The `git` loader renders the last commits touching a path (`git log`), the most recent tags (`git tags`) or the authors of a path (`git contributors`). Its snapshot is taken by running the same `git` query that renders it, so `check` runs the `git` binary. This amends [ADR 0006](0006-check-never-runs-a-loader.md). `check` still never runs a command from the repository; it may run `git` to read history, because reading history runs no repository code.

The snapshot is what the text is made from, one entry per line: the full commit ids for `log`, and the names in order for `tags` and `contributors`.

## Context

ADR 0006 made `check` compare sums without running a loader, for two reasons: running an unvetted clone's commands to answer a yes-or-no question is unsafe, and running every generator on every commit is slow. `tree` was already the exception in form, not in spirit: its snapshot and its listing are one walk. Nothing it does is controlled by the repository beyond the names it lists.

History lives in `.git`, which a wildcard in `inputs=` never selects ([ADR 0012](0012-wildcards-in-inputs-do-not-reach-ignored-paths.md)) and which no glob could snapshot honestly anyway, with packed refs, reftables and linked worktrees. Before this loader, every region derived from history had to be `volatile`, and `check` passes a volatile region without looking. The site's catalogue named that as the strongest argument for a `git` loader.

## Considered Options

- **Keep history `volatile`.** No new rule. Rejected: `check` cannot see a changelog, a contributor list or a tag list go out of date, which is the only thing the tool is for.
- **Snapshot `HEAD` by reading `.git` files directly,** with no process. Rejected: every commit would make every `git` region stale, including `git log src=docs/` when the commit touched nothing under `docs/`. It would also mean reimplementing ref resolution for packed refs, reftables and worktrees.
- **Read the object store in-process** with a Rust git library. No subprocess, so ADR 0006 would stand unamended. Rejected: `libgit2` bindings need a C library, and the pure-Rust option is a large dependency for three queries. Either one has to match `git`'s own `.mailmap` handling and path limiting exactly, or the output would differ from what people see in `git log`.
- **Run the same `git` query in the snapshot step.** Chosen.

## Why this is not running repository code

The `git` binary is the user's and its configuration is the user's. A clone's tracked content cannot configure it: `.git/config` is written by `git clone` on this machine and is not fetched from the remote. The tracked files these queries read, such as `.mailmap`, are data. The loader closes the routes by which configuration could run a command or reshape the output:

- `--no-pager`, `-c core.fsmonitor=false` and `-c log.showSignature=false`, so no pager, fsmonitor hook or signature program starts;
- `--no-replace-objects`, `--literal-pathspecs` and `--no-optional-locks`, and fixed `log.follow` and output encoding;
- `LC_ALL=C`, `TZ=UTC` and `GIT_TERMINAL_PROMPT=0`, with `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_CONFIG_PARAMETERS` and the other variables that redirect `git` removed, since a pre-commit hook sets some of them.

The user's and the system's configuration still load, so `safe.directory` keeps working in CI containers.

## Consequences

- `check` spawns one `git` process per `git` region. It is still fast, and it is no longer free of subprocesses.
- A shallow clone is a hard error for the region, with the fix in the message (`fetch-depth: 0` in `actions/checkout`). CI that checks a `git` region needs the full history.
- `check` now depends on `git` being installed when a template has a `git` region. Outside a repository, a `git` region is an error.
- A tag that moves to another commit is not a change, because the output shows names only. `contributors` only ever appends, in order of first commit, so a new commit by a known author changes nothing.
- `affected` treats `git log` and `git contributors` as reached by paths under `src=`, and `git tags` as reached by no path.
- ADR 0006's rule now reads "`check` never runs a command from the repository". The trust rule in [ADR 0017](0017-trust-gates-running-repository-code-and-nothing-else.md) draws the same line.
