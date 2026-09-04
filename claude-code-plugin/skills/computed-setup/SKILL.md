---
name: computed-setup
description: Install the computed CLI and wire it into a repository, the pre-commit hook and the CI check. Use when setting computed up in a project, when `computed` is missing or unwired, or when acting on what `computed run` or `computed check` reported. Writing the regions themselves is `discover-regions`, offered at the end.
---

# computed-setup

`computed` owns the span between two comment markers in a Markdown file and rewrites it when the inputs it was computed from move. This skill puts the tool in place and gives the repository something that catches drift. Finding blocks worth owning and writing the markers is `discover-regions`, offered in step 4.

Setup runs as survey, ask, apply. It is done when `computed --version` answers, `computed check` exits 0, and the enforcement the user chose is in place.

The marker grammar, the region states and the exit tiers live in [`REFERENCE.md`](../REFERENCE.md). Read it before acting on any state `run` or `check` reports.

## 1. Survey

Answer these from the repository itself. Every one you answer here is a question the wizard does not have to ask.

| Question | How |
|---|---|
| Is `computed` installed? | `computed --version` |
| Is `cargo` available? | `cargo --version` |
| Where is the repository root? | `git rev-parse --show-toplevel` |
| What already enforces anything? | `.pre-commit-config.yaml`, `.github/workflows/*.yml`, `.git/hooks/pre-commit` |
| Are there regions already? | `grep -rl '<!-- computed' --include='*.md' .` |

## 2. Ask

One `AskUserQuestion` call carrying only what the survey left open. Drop any question the user already answered in their request. Put your recommendation first and mark it `(Recommended)`.

**Install**, only when `computed --version` failed:

- `cargo install computed`, recommended when `cargo --version` succeeded
- A prebuilt binary from the [latest release](https://github.com/mitchellvanw/computed-files/releases/latest), for macOS or Linux with no toolchain

**Enforcement**, how drift gets caught:

- A pre-commit hook and a CI check (Recommended)
- A pre-commit hook only
- A CI check only
- Neither, run `computed run` by hand

## 3. Apply

1. **Install**, if the survey found none.
2. **Wire the enforcement chosen.**

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

   `always_run` and `pass_filenames: false` both matter. A staged change under `src/` makes a region stale without staging the file that holds it, so a hook that runs only on staged Markdown never fires. Without the pre-commit framework, `.git/hooks/pre-commit` is two lines:

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
3. **Verify.** `computed check` exits 0. A region that already existed and reports `stale` or `edited` is drift the survey turned up, not a broken setup. [`REFERENCE.md`](../REFERENCE.md) says what each state wants.

## 4. Hand off

The repository now has the tool and something that catches drift. It has no regions unless it already had some, so ask once with `AskUserQuestion`.

**Look for blocks worth computing?**

- Yes, look now (Recommended). Run the `discover-regions` skill.
- Not now. Tell them `/discover-regions` does it whenever they want.

Report what was installed, what now catches drift, and which of the two they chose.
