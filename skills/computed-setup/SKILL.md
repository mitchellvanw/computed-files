---
name: computed-setup
description: Set up the computed CLI in a repository and act on what it reports. Use when installing or wiring computed into a project, adding a `<!-- computed -->` region, or working out what `computed run` or `computed check` just said — stale, edited, untrusted, or a loader failure.
---

# computed

`computed` owns the span between two comment markers in a Markdown file and rewrites it when the inputs it was computed from move. Edit the prose around a region; leave the body to `computed run`.

Setup runs as three steps: **survey**, **ask**, **apply**. It is done when `computed run` writes nothing and `computed check` exits 0, with the enforcement the user picked in place.

Grammar, every attribute, the region states and the exit tiers live in [`REFERENCE.md`](REFERENCE.md). Read it before writing a marker that is not one of the two templates in step 3, and before acting on any state `run` or `check` reports.

## 1. Survey

Answer these from the repository itself. Every one you answer here is a question the wizard does not have to ask.

| Question | How |
|---|---|
| Is `computed` installed? | `computed --version` |
| Is `cargo` available? | `cargo --version` |
| Where is the repository root? | `git rev-parse --show-toplevel` |
| What already enforces anything? | `.pre-commit-config.yaml`, `.github/workflows/*.yml`, `.git/hooks/pre-commit` |
| Which regions exist already? | `grep -rl '<!-- computed' --include='*.md' .` |
| What is worth owning? | Hand-written blocks that go stale on their own: a file tree, a command's output, an index, a list of decisions — usually in `CLAUDE.md` or `README.md`. Read the candidates before offering them. |

## 2. Ask

One `AskUserQuestion` call carrying only what the survey left open, and only what changes the work. Drop any question the user already answered in their request. Put your recommendation first and mark it `(Recommended)`.

**Files** — which Markdown files computed should own a region in. `multiSelect: true`, offering the candidates the survey found, most stale-looking first.

**Region** — what the first region should hold:

- A file tree of the repository — the `tree` loader, no trust needed
- The output of a command — the `exec` loader, needs `computed trust`
- Both

**Enforcement** — how drift gets caught:

- A pre-commit hook and a CI check (Recommended)
- A pre-commit hook only
- A CI check only
- Neither, run `computed run` by hand

**Install** — only when `computed --version` failed:

- `cargo install computed` — recommend this when `cargo --version` succeeded
- A prebuilt binary from the [latest release](https://github.com/mitchellvanw/computed-files/releases/latest), for macOS or Linux with no toolchain

## 3. Apply

1. **Install**, if the survey found none.
2. **`computed trust`** — once per clone, only when an `exec` region was chosen. Until a grant exists, `run` skips exec regions, keeps their bodies and exits 1.
3. **Write the markers.** Put each region where the hand-written block sits, replacing that block: opener, empty body, bare closer. `run` fills the body and writes the sums.

   ~~~markdown
   <!-- computed tree src=. depth=2 name=layout -->
   <!-- /computed -->
   ~~~

   ~~~markdown
   <!-- computed exec cmd="<command>" inputs=<glob>,<glob> name=<name> as=fence -->
   <!-- /computed -->
   ~~~

   An exec region takes exactly one of `inputs=` or `volatile`. Point `inputs=` at the files whose change should make the region stale — that is what lets `check` do its job without running the command.

4. **Render and verify.** `computed run` exits 1 the first time because it wrote a file. Run it again: it writes nothing and exits 0. Then `computed check` exits 0. That is the completion bar; do not stop before it.
5. **Wire the enforcement chosen.**

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

6. **Report** which files gained regions, what each one computes, and what now catches it when they drift.
