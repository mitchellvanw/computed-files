---
status: accepted
---

# Exec trust is granted per clone, outside the working tree

An exec region runs only when the repository it sits in has been trusted on this machine. The grant is recorded by `computed trust` in `~/.config/computed/trust.toml` (honouring `XDG_CONFIG_HOME`), keyed by the repository's canonical root path, and never by anything inside the working tree. `computed run --trust` grants for one invocation without writing the file. In an untrusted repository, `run` skips every exec region, keeps its body, reports it `untrusted`, still renders tree regions, and exits non-zero. `check` never runs a loader, so trust never enters it.

The model is direnv's and git's `safe.directory`, and it defends against one thing: cloning a repository and having `computed run` execute its commands before anyone read them. A malicious pull request branch, or a dependency that writes markers into a file, runs on the next `run` of a trusted clone exactly as a Makefile or pre-commit hook would; the spec says so rather than implying a guarantee it cannot keep.

## Considered Options

- **A trust file committed in the repository.** Rejected: the attacker controls it, so it is a marker, not a grant.
- **A flag on every invocation.** Rejected: it gets pasted into the pre-commit config and becomes a committed file.
- **An interactive prompt on first run** (org-mode). Rejected: pre-commit and CI have no TTY, and the hook path is the reason the tool exists.
- **A per-region command allowlist.** Rejected: it re-prompts on every edit and nobody reads the commands anyway.
- **Hashing the exec commands into the grant** (direnv re-allows on `.envrc` change). Rejected: it turns every `git pull` into a re-trust ceremony while only appearing to defend against the pull-request case.
- **The grant in `.git/config`.** Rejected: leaves a file outside a repository with no way to be trusted, and needs git config parsing.
- **A grant per canonical repository root in the user's config directory.** Chosen. Cost: a moved or re-cloned repository is untrusted again, and a submodule is its own repository with its own grant.

## Consequences

- Trust is looked up per template file against its own repository root (the `COMPUTED_ROOT` already computed for exec's environment), or its region root outside a repository. Symlinks are resolved on write and on lookup.
- An untrusted exec region reuses the loader-failure rule: body kept, nothing written into it, run exits non-zero. A pre-commit hook therefore cannot pass with silently stale content.
- A `check`-only CI pipeline never touches the trust model. A pipeline that runs `run` or `run --dry-run` passes `--trust`.
- The tree loader's `src=` and exec's `inputs=` must resolve inside the repository root (or region root outside one); escaping is a hard error. This is a reproducibility rule, not a security fence: exec itself is unfenced because it is trusted.
