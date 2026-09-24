---
name: computed-setup
description: Install the computed CLI and wire it into a repository, the pre-commit hook and the CI check. Use when setting computed up in a project, when `computed` is missing or unwired, or when acting on what `computed run` or `computed check` reported. Writing the regions themselves is `discover-regions`, offered at the end.
---

# computed-setup

`computed` owns the span between two comment markers in a Markdown or code file and rewrites it when the inputs it was computed from move. This skill puts the tool in place and gives the repository something that catches drift. Finding blocks worth owning and writing the markers is `discover-regions`, offered in step 4.

Setup runs as survey, ask, apply. It is done when `computed --version` answers, `computed check` exits 0, and the enforcement and extras the user chose are in place.

The marker grammar, the region states and the exit tiers live in [`REFERENCE.md`](../REFERENCE.md). Read it before acting on any state `run` or `check` reports.

## 1. Survey

Answer these from the repository itself. Every one you answer here is a question the wizard does not have to ask.

| Question | How |
|---|---|
| Is `computed` installed? | `computed --version` |
| Is `cargo` available? | `cargo --version` |
| Where is the repository root? | `git rev-parse --show-toplevel` |
| What already enforces anything? | `.pre-commit-config.yaml`, `.github/workflows/*.yml`, `.git/hooks/pre-commit` |
| Is the repository on GitHub? | `git remote -v` |
| Are there regions already? | `grep -rl '<!-- computed' --include='*.md' .` |
| Is the merge driver installed? | `git config merge.computed.driver`, and `merge=computed` in `.gitattributes` |
| Are the guard hooks in the project settings? | `computed guard --hook` in `.claude/settings.json` |

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

**Extras**, with `multiSelect: true`. Offer only what the survey found missing:

- **The merge driver.** Two branches that each re-render a region conflict inside it; the driver resolves those and leaves the region for the next `run`. Recommend it when the repository has a `tree` or `index` region, which re-render on most branches.
- **The guard hooks in `.claude/settings.json`.** With this plugin installed they already run in this session. In the project settings they also guard every teammate's agent that lacks the plugin: an edit to a region body is refused before it lands, and drift after an edit is reported.
- **Pull request suggestions,** on GitHub only. The Action's `suggest` command posts what `computed run` would write as review suggestions on each pull request.

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

   CI on GitHub, through computed's Action, which installs a checked release and annotates every region that is not fresh:

   ```yaml
   - uses: actions/checkout@v4
   - uses: mitchellvanw/computed-files@main
   ```

   CI anywhere else:

   ```yaml
   - run: cargo install computed
   - run: computed check
   ```

   `check` runs no command from the repository, so a `check`-only pipeline needs no trust grant on the runner. A pipeline that runs `run` passes `--trust` for that one invocation. A repository with a `git` region needs its full history checked out (`fetch-depth: 0`).
3. **Apply the extras chosen.**

   The merge driver: `computed merge --install`. It adds `*.md` and `*.markdown` lines to `.gitattributes`, which get committed; code files with regions each need a line such as `*.rs merge=computed` by hand. It also sets the driver in this clone's git config, which every clone does once, as with `computed trust`.

   The guard hooks, merged into `.claude/settings.json` without disturbing what is there:

   ```json
   {
     "hooks": {
       "PreToolUse": [
         { "matcher": "Edit|MultiEdit|Write",
           "hooks": [{ "type": "command", "command": "computed guard --hook pre" }] }
       ],
       "PostToolUse": [
         { "matcher": "Edit|MultiEdit|Write",
           "hooks": [{ "type": "command", "command": "computed guard --hook post" }] }
       ]
     }
   }
   ```

   Pull request suggestions, as a workflow job:

   ```yaml
   permissions:
     contents: read
     pull-requests: write
   steps:
     - uses: actions/checkout@v4
       with:
         ref: ${{ github.event.pull_request.head.sha || github.sha }}
     - uses: mitchellvanw/computed-files@main
       with:
         command: suggest
   ```

   `suggest` runs no exec region unless the job passes `trust: "true"`. Pass it only for a repository whose pull requests CI already runs the code of.
4. **Verify.** `computed check` exits 0. A region that already existed and reports `stale` or `edited` is drift the survey turned up, not a broken setup. [`REFERENCE.md`](../REFERENCE.md) says what each state wants.

## 4. Hand off

The repository now has the tool and something that catches drift. It has no regions unless it already had some, so ask once with `AskUserQuestion`.

**Look for blocks worth computing?**

- Yes, look now (Recommended). Run the `discover-regions` skill.
- Not now. Tell them `/discover-regions` does it whenever they want.

Report what was installed, what now catches drift, which extras are in place, and which of the two they chose. The Action, the merge driver, the hooks and the editor setup are written up in [`docs/integrations.md`](https://github.com/mitchellvanw/computed-files/blob/main/docs/integrations.md).
