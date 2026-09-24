# Action self-test

The `release` job in [`workflows/action-selftest.yml`](workflows/action-selftest.yml) runs the GitHub Action with the latest released `computed` and checks this file. `CLAUDE.md` cannot serve: its regions use loaders newer than the release, and a release parses the whole file or none of it.

Write regions here only in grammar the latest release reads. The repository's own `computed run` keeps them fresh, so when the job fails here, the release and this checkout disagree about a sum: a bug, or a format constant bumped since the release, which the next release settles.

## The action's scripts

<!-- computed tree src=../scripts/action name=scripts | do not edit; run computed -->
```
.
├── blocks.awk
├── hunks.awk
├── install.sh
├── ranges.awk
└── run.sh
```
<!-- /computed in=18fc6bfb753e9341af430b0cb6733ccd4e0b75c569ae518ea8b03beb6bdd2788 out=90f6164dd8ea5fd5d6d01e9ffe8ca1a86068bb4672f9fc20a0a188d005660a7c -->

## The action

<!-- computed file src=../action.yml name=action as=fence lang=yaml | do not edit; run computed -->
```yaml
name: computed
description: Check that computed regions are fresh, or suggest on a pull request what `computed run` would write.
author: Mitchell van Wijngaarden
branding:
  icon: refresh-cw
  color: gray-dark

inputs:
  version:
    description: >-
      The computed release to install: `latest`, or a tag such as `v0.2.0`.
      `installed` skips the download and uses the computed already on PATH.
    default: latest
  command:
    description: >-
      `check` reports every region that is not fresh and fails on drift.
      `suggest` also posts, on a pull request, what `computed run` would
      write: review suggestions on the lines the pull request touched, one
      summary comment for the rest. It needs `pull-requests: write`, and the
      pull request's head commit checked out for inline suggestions.
    default: check
  paths:
    description: Space-separated files and directories to process. Empty means the repository, as `computed` discovers it.
    default: ""
  trust:
    description: >-
      `true` runs exec regions under `suggest` (`computed run --trust`). Only
      for repositories whose pull requests you would run the commands of
      anyway. `check` never runs a command and ignores it.
    default: "false"
  allow:
    description: >-
      Space-separated url prefixes `remote` regions may fetch under during
      `suggest` (`computed run --allow`), for this run only; the runner keeps
      no allowlist. `check` never fetches and ignores it.
    default: ""
  token:
    description: The token `gh` downloads the release and posts comments with.
    default: ${{ github.token }}

outputs:
  version:
    description: The computed version that ran.
    value: ${{ steps.install.outputs.version }}

runs:
  using: composite
  steps:
    - id: install
      shell: sh
      env:
        INPUT_VERSION: ${{ inputs.version }}
        GH_TOKEN: ${{ inputs.token }}
      run: sh "$GITHUB_ACTION_PATH/scripts/action/install.sh"
    - shell: sh
      env:
        INPUT_COMMAND: ${{ inputs.command }}
        INPUT_PATHS: ${{ inputs.paths }}
        INPUT_TRUST: ${{ inputs.trust }}
        INPUT_ALLOW: ${{ inputs.allow }}
        GH_TOKEN: ${{ inputs.token }}
      run: sh "$GITHUB_ACTION_PATH/scripts/action/run.sh"
```
<!-- /computed in=7d2b994a70b790f546c09a9e386f457f8d331a6066c3fc3f66d1462dac522540 out=cf39167b542c16e0b86ec155cd6f4a31bc649e24e53d67defac5e86700293465 -->
