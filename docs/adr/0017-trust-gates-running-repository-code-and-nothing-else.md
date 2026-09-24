---
status: accepted
---

# Trust gates running repository code, and nothing else

A loader needs trust exactly when it runs a command the repository controls. That is `exec` and `transcript`. Every other loader is native: `tree`, `file`, `value`, `index`, `toc` and `symbol` read files, `git` reads history through the `git` binary, and `remote` fetches a pinned document. None of them needs a grant. `render::needs_trust` is the one test, and render, the guard's hint and the language server all ask it. A `use` region is asked by the loader its recipe expands to.

The loader set grows by native loaders written into the closed enum, not by a plugin interface, and `exec` stays the escape hatch for everything else.

## Context

Until 0.2 there were two loaders that mattered: `tree`, and `exec` for everything else. An ADR index, a version string, a table of contents, one function's signature and the last ten commits were all scripts behind `exec`. On a fresh clone every one of them is `untrusted`: `run` skips it and exits 1, and the pre-commit hook fails until someone runs `computed trust`, for regions that execute nothing but a read. [ADR 0015](0015-the-file-loader.md) made this argument once, for `file`. Eight new loaders arrived together, and each needed the same answer, so the rule itself had to be written down.

[ADR 0007](0007-exec-trust-per-clone.md) defends against one thing: cloning a repository and having `computed run` execute its commands before anyone read them. The question for each loader is whether it can be made to do that.

## Considered Options

- **Everything but `tree` and `file` needs trust.** The most conservative line. Rejected: `symbol`, `value` and `index` only read files, just as `file` does, and a fresh clone would fail its hook on them.
- **Anything that spawns a process needs trust.** A line that is easy to check in the code. Rejected: it draws the line at the mechanism, not at who controls the program. `git` is the user's binary under the user's configuration, and a clone controls only the data it reads ([ADR 0018](0018-the-git-snapshot-runs-git-under-check.md)).
- **Capabilities per loader,** with a grant naming the ones it allows. Rejected for the reason ADR 0007 rejected a per-region allowlist: it asks again on every edit, and nobody reads what they grant.
- **Put `remote` under trust.** Rejected: a grant answers "may this clone run code here", and a fetch asks something else, whether this machine may send a request to that host. [ADR 0019](0019-remote-regions-are-pinned-and-allowlisted.md) gives it a gate of its own.
- **Loaders as external programs** behind a plugin interface. Anyone could add one. Rejected: an external loader is a command, so it needs trust, and its determinism and snapshot are out of the tool's hands. A variant in the enum comes with a reviewed snapshot definition and a format constant.
- **Trust gates running repository-controlled commands.** Chosen. It is the line ADR 0007 already drew, applied to every loader.

## Consequences

- A fresh clone renders every native region without a grant. Only `exec` and `transcript` regions report `untrusted`.
- `transcript` needs trust whatever its `workdir=`: its steps are commands from the file.
- Each native loader is a snapshot definition and a format constant to keep. A wrong snapshot is a false fresh or a false stale, which `exec` with `inputs=` got wrong in the author's script rather than in the tool.
- "Reading" has to stay true in the code. The `git` loader turns off the configuration keys that would run a command (ADR 0018), and `symbol` parses with tree-sitter grammars compiled into the binary.
- The binary grew from 3.2 MB to 8.2 MB, most of it the three tree-sitter grammars, rustls and ring.
- The `sandbox` flag does not move this line: a sandboxed `exec` region still needs trust ([ADR 0020](0020-the-sandbox-enforces-inputs-and-does-not-replace-trust.md)).
