---
status: accepted
---

# Rust for the prototype

We considered Elixir and Rust for the first implementation. The research concluded that the daemon is a convenience layer and the foundation is `run` and `check` in pre-commit hooks and CI, on machines we do not control. That path wants a single static binary with millisecond startup and no runtime to install, which Rust gives and an Elixir release does not. Two crates map directly onto the hardest parts: `notify` for watching and `ignore` for gitignore-aware directory walking, which the tree loader needs and Elixir has no equivalent for.

## Considered Options

- **Elixir.** OTP supervision models the daemon well: one process per file gives single-flight, a supervisor keeps the last good region on a crash, settle windows are timer messages. Rejected because the daemon is not the product, file-watching libraries wrap platform binaries through ports, and distribution ships the BEAM.
- **Rust.** Chosen for the reasons above, and because the developer's tooling already targets Rust workspaces.
