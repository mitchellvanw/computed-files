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

## Loaders

| Loader | Attributes | Default sink |
|---|---|---|
| `tree` | `src=.`, `depth=`, and the flags `all` and `dirs` | `fence` |
| `exec` | `cmd=`, `timeout=30`, and exactly one of `inputs=` or `volatile` | `raw` |

Every region also takes `name=` for stable reports, `as=raw|fence` to pick the sink, and `lang=` for the fence language. Anything after a `|` in the opener is a note for human readers.

`inputs=` is a comma-separated list of globs, and it is what makes an exec region checkable: `check` re-snapshots those paths and compares sums without running the command. `volatile` declares there is nothing worth snapshotting, so the region re-renders on every `run` and `check` always passes it.

Relative paths in a marker resolve against the directory of the file that holds the marker, and an exec command runs there, not in the repository root and not in the shell's working directory. An exec command gets `COMPUTED_ROOT`, `COMPUTED_FILE` and `COMPUTED_REGION` in its environment and runs under `LC_ALL=C`, `TZ=UTC` and an empty `LANGUAGE`, so it gives the same bytes on a laptop and in CI. `tree` follows `.gitignore`.

An exec region runs only in a clone `computed trust` has granted, recorded in `~/.config/computed/trust.toml` and never in the working tree. Cloning a repository therefore executes nothing in it.

## Commands

<!-- computed exec cmd=../../scripts/cli-commands.sh inputs=../../src/cli.rs,../../scripts/cli-commands.sh name=commands as=fence | do not edit; run computed -->
```
computed run      [paths] [--force] [--dry-run] [--trust]
computed check    [paths]
computed clean    [paths] [--force] [--dry-run]
computed trust    [path]
computed untrust  [path]
```
<!-- /computed in=f711e7db08a6fb99dda2f6d04527f6502a9aca33cff2fe8e31afd78897e42bf9 out=afa1d18e99f8290357f2309dcdf34bb74cf49bbf8acadcaab7bad3967715bbc8 -->

With no paths, the current directory is walked with the tree loader's ignore settings and every `.md` file is read. An explicit file is read whatever its extension. `run --dry-run` prints the diff `run` would write and writes nothing.

## States

One line per region goes to stderr, as `path:line name loader state`. Fresh regions print only under `-v`.

| State | Meaning | `run` | `check` |
|---|---|---|---|
| `fresh` | Both sums match. | leaves it alone | passes |
| `stale` | Inputs or the opener changed. | re-renders | exit 1 |
| `edited` | The body was changed by hand. | refuses the file, exit 1 | exit 1 |
| `volatile` | Declares no inputs. | re-renders every time | passes |
| `unrendered` | The closer carries no sums. | renders | exit 1 |

| Exit | Meaning |
|---|---|
| 0 | Everything is fresh. |
| 1 | The content said no: drift under `check`; a write, a refused file, a loader failure or an untrusted region under `run`. |
| 2 | The tool could not answer: usage error, marker parse error, a path escaping the repository, `inputs=` matching nothing. |

## Acting on a state

- **`stale`**. Run `computed run`.
- **`edited`**. The body no longer matches `out=`. `run` refuses the whole file, leaves every region in it untouched and exits 1. Keep the hand-written change somewhere if it was wanted, then `computed run --force` hands the body back to the tool.
- **`untrusted`**. An exec region in a clone with no grant. Run `computed trust`, or pass `run --trust` for a single invocation.
- **A loader failure**. The command's stderr prints under the region's line, the last good body and sums stay put, and the run exits 1. Repair the input and run again.
