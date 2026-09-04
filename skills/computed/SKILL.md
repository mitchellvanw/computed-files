---
name: computed
description: Keep generated spans of a Markdown file current with the computed CLI. Use when wiring computed into a repository, writing a `<!-- computed -->` region marker, or acting on what `computed run` or `computed check` reported — stale, edited, untrusted, or a loader failure.
---

# computed

`computed` owns the span between two comment markers in a Markdown file and rewrites it when the inputs it was computed from move. The prose around a region belongs to whoever is writing the file. The body between the markers belongs to the tool: edit the prose, and leave the body to `computed run`.

Source, spec and decisions: <https://github.com/mitchellvanw/computed-files>

## Install

```
cargo install computed
```

Without a Rust toolchain, take a prebuilt macOS or Linux binary from the [latest release](https://github.com/mitchellvanw/computed-files/releases/latest) and put it on `PATH`.

## Wire it into a repository

Done when `computed check` exits 0.

1. `computed trust` — once per clone. Only `exec` regions need it; until a grant exists `run` skips them, keeps their bodies and exits 1.
2. Add a region to the file that keeps going stale, usually `CLAUDE.md` or `README.md`. Write the opener and a bare closer and leave the body empty.
3. `computed run` — fills the body and writes both sums into the closer. It exits 1 because it wrote a file.
4. `computed check` — exits 0.
5. Add the hook and the CI step.

`.pre-commit-config.yaml`:

```yaml
repos:
  - repo: local
    hooks:
      - id: computed
        name: computed run
        entry: computed run
        language: system
        always_run: true
        pass_filenames: false
```

`always_run` and `pass_filenames: false` carry the weight: a staged change under `src/` makes a tree region stale without staging the file that holds it. Without the pre-commit framework, `.git/hooks/pre-commit` is two lines:

```sh
#!/bin/sh
exec computed run
```

CI:

```yaml
- run: cargo install computed
- run: computed check
```

`check` runs no loaders, so a `check`-only pipeline needs no trust grant on the runner. A pipeline that runs `run` passes `--trust` for that one invocation.

## Write a region

~~~markdown
<!-- computed tree src=. depth=2 name=layout -->
<!-- /computed -->
~~~

`run` fills the body and rewrites the closer with the two sums:

~~~markdown
<!-- /computed in=4b0267a3…c898 out=45ca178e…f559 -->
~~~

`in=` covers the canonical opener plus a snapshot of the region's inputs, so it answers "did the inputs move". `out=` covers the body the tool wrote, so it answers "did someone edit this by hand". Both are full SHA-256.

| Loader | Attributes | Default sink |
|---|---|---|
| `tree` | `src=.`, `depth=`, and the flags `all` and `dirs` | `fence` |
| `exec` | `cmd=`, `timeout=30`, and exactly one of `inputs=` or `volatile` | `raw` |

Every region also takes `name=` for stable reports, `as=raw|fence` to pick the sink, and `lang=` for the fence language. Anything after a `|` in the opener is a note for human readers.

`inputs=` is a comma-separated list of globs, and it is what makes an exec region checkable — `check` re-snapshots those paths and compares. `volatile` declares there is nothing worth snapshotting, so the region re-renders on every `run` and `check` always passes it.

Relative paths in a marker resolve against the directory of the file that holds the marker, and an exec command runs there — not the repository root, not the shell's working directory. An exec command gets `COMPUTED_ROOT`, `COMPUTED_FILE` and `COMPUTED_REGION` in its environment, and runs under `LC_ALL=C`, `TZ=UTC` and an empty `LANGUAGE`. `tree` follows `.gitignore`.

Markers inside a fenced code block are prose, so an example like the ones above renders nothing.

## Commands

<!-- computed exec cmd=../../scripts/cli-commands.sh inputs=../../src/cli.rs,../../scripts/cli-commands.sh name=commands as=fence | do not edit; run computed -->
```
computed run      [paths] [--force] [--dry-run] [--trust]
computed check    [paths]
computed clean    [paths] [--force] [--dry-run]
computed trust    [path]
computed untrust  [path]
```
<!-- /computed in=f720acd6bd9e5cb764674b19f4f77b3c8a59ea5930369c8eebf45af8d655e444 out=afa1d18e99f8290357f2309dcdf34bb74cf49bbf8acadcaab7bad3967715bbc8 -->

With no paths, the current directory is walked with the tree loader's ignore settings and every `.md` file is read. An explicit file is read whatever its extension. `run --dry-run` prints the diff `run` would write and writes nothing.

## Reading a report

One line per region goes to stderr; fresh regions print only under `-v`.

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

## When a run does not come back clean

- **`stale`** — run `computed run`.
- **`edited`** — the body no longer matches `out=`. `run` refuses the whole file, leaves every region in it untouched and exits 1. Keep the hand-written change if it was wanted, then `computed run --force` to hand the body back to the tool.
- **`untrusted`** — an exec region in a clone with no grant. Run `computed trust`, or pass `run --trust` for a single invocation.
- **a loader failure** — the command's stderr prints under the region's line, the last good body and sums stay put, and the run exits 1. Repair the input and run again.
