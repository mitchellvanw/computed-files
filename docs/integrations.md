# Wiring computed in

`computed run` in a pre-commit hook and `computed check` in CI are the foundation; the [README](../README.md#hooks) sets those up. This page covers the rest: the GitHub Action, the merge driver, the Claude Code hooks, and editors. What each command does is in the [spec](spec/computed-v0.md#what-is-the-command-line).

## GitHub Action

The repository root is a composite action. It installs a release, checks the archive against the release's `SHA256SUMS`, and runs one of two commands.

```yaml
jobs:
  computed:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write   # only for command: suggest
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ github.event.pull_request.head.sha || github.sha }}
      - uses: mitchellvanw/computed-files@main
        with:
          command: suggest
```

| Input | Default | Meaning |
|---|---|---|
| `version` | `latest` | A release tag such as `v0.2.0`, or `installed` to use the `computed` already on `PATH`. |
| `command` | `check` | `check` fails on drift and annotates each region that is not fresh. `suggest` also posts, on a pull request, what `computed run` would write. |
| `paths` | empty | Space-separated files and directories. Empty means the repository, as `computed` discovers it. |
| `trust` | `"false"` | `"true"` runs exec and transcript regions under `suggest`. Only for repositories whose pull requests you would run the commands of anyway. `check` runs no command and ignores it. |
| `allow` | empty | Space-separated url prefixes `remote` regions may fetch under during `suggest`. The runner keeps no allowlist. |
| `token` | `github.token` | What `gh` downloads the release and posts comments with. |

`suggest` turns each change `run` would make inside a line the pull request touched into a review suggestion on the head commit, so it needs the head commit checked out, as above. Changes elsewhere go into one summary comment, edited in place on later pushes. On a fork's pull request the token cannot comment, and the action says so in a warning. Either command exits as `computed` does, so drift fails the job.

Runners: Linux on x86-64 and arm64, macOS on Apple silicon and Intel.

## Merge driver

Two branches that each re-render a region conflict inside it, over a body neither side should resolve by hand. The merge driver takes the side that changed a region, and leaves a region both sides re-rendered unrendered for the next `run`:

```
computed merge --install
```

It adds `*.md merge=computed` and `*.markdown merge=computed` to the root `.gitattributes`, which you commit, and sets `merge.computed.driver` in this clone's git config, which every clone does once, as with `computed trust`. A clone that has not installed it merges these files line by line, as before. After a merge, `computed check` exits 1 until `computed run` renders the regions left unrendered. The reasoning is [ADR 0024](adr/0024-the-merge-driver-leaves-doubly-rendered-regions-unrendered.md).

## Claude Code

The plugin registers `computed guard` as three hooks. Before an `Edit`, `MultiEdit` or `Write` lands, it refuses an edit that changes a region's body or closer and tells the agent which source to edit instead. After the edit, it runs `check` on the file and hands any drift back to the agent. At session start, it says so if `computed` is missing from `PATH`. The hooks need a `computed` with the `guard` command on `PATH`, and allow every edit when there is none.

```
/plugin marketplace add mitchellvanw/computed-files
/plugin install computed@computed
/reload-plugins
```

Without the plugin, put the two edit hooks in `.claude/settings.json`:

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

An edit made through the shell, such as `sed -i`, is not seen by these hooks; the pre-commit hook still refuses it. Another agent or editor can ask the same question with `computed guard FILE --proposed PATH`, which exits 1 when the edit touches a region. The reasoning is [ADR 0025](adr/0025-the-guard-refuses-an-edit-before-it-lands.md).

## Editors

`computed lsp` is a language server on stdin and stdout. It shows each region's state as a diagnostic while you type, the region's opener, inputs and sums on hover, and a code lens on each opener that renders the buffer as `run` would. The render arrives as an edit to the buffer, not a write to disk, so it can be undone and is saved when you save.

**Neovim** 0.11 or later:

```lua
vim.lsp.config('computed', {
  cmd = { 'computed', 'lsp' },
  filetypes = { 'markdown' },
  root_markers = { '.git' },
})
vim.lsp.enable('computed')
-- Code lenses: call vim.lsp.codelens.refresh() on BufEnter and InsertLeave,
-- and run one with vim.lsp.codelens.run().
```

**Helix**, in `languages.toml`. Diagnostics and hover work; Helix has no code lens, and `:lsp-workspace-command` sends no argument, so the run command does nothing there.

```toml
[language-server.computed]
command = "computed"
args = ["lsp"]

[[language]]
name = "markdown"
language-servers = ["marksman", "computed"]
```

**VS Code** has no generic language client, so it takes a small extension built on `vscode-languageclient`, with `"activationEvents": ["onLanguage:markdown"]` in its `package.json`:

```js
const { LanguageClient } = require('vscode-languageclient/node');
exports.activate = () => new LanguageClient('computed', 'computed',
  { command: 'computed', args: ['lsp'] },
  { documentSelector: [{ scheme: 'file', language: 'markdown' }] }).start();
```

Inputs that change on disk are noticed on the next open, change or save of the document.

## Watching

`computed watch [paths]` runs `run` again whenever a template or a file its regions read changes, until Ctrl-C. It takes `--trust` and `--allow` as `run` does. A session where an agent reads `CLAUDE.md` between commits is where it pays: the regions it reads are current without waiting for the pre-commit hook.
