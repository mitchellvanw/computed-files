---
status: accepted
---

# The sandbox enforces `inputs=` and does not replace trust

An `exec` or `transcript` region that carries the `sandbox` flag runs its command in an operating-system sandbox. The command may read its declared input files, the system's program and library trees, and the `PATH` directories outside the repository. It may list any directory inside the repository and stat anything. It may write only to a fresh `TMPDIR` and `/dev/null`, and it reaches no network. A read or write outside that fails the command, so the region fails and keeps its body: `inputs=` is complete, or the command does not run.

A sandboxed region still needs trust. Where no sandbox is available the region is an `error` (tier 2); it never runs unsandboxed. On macOS the sandbox is `sandbox-exec` with a deny-default profile and no Mach lookups. On Linux it is Landlock (kernel 5.13 or later) for the file system and a seccomp filter that makes `socket(2)` and `io_uring_setup(2)` fail, both applied between fork and exec.

## Context

`check` trusts `inputs=` to name everything a command reads ([ADR 0006](0006-check-never-runs-a-loader.md)). An undeclared read is a false fresh. The command's output changes, the region reads `fresh`, and nothing notices until `run --force`. Nothing enforced the declaration. `trace` can report what a command read, but only when someone runs it.

A sandbox that denies undeclared reads could serve two purposes. It could make `inputs=` enforced, which is a correctness property. Or it could let an untrusted clone run its commands safely, which is a security property. They pull in different directions: the first wants a command to keep working with everything it legitimately needs, and the second wants as little as possible allowed.

## Considered Options

- **The sandbox replaces trust:** an untrusted clone may run sandboxed regions. Fresh clones would render more. Rejected: the sandbox allows reading and executing every system tree and `PATH` directory, and the command's output is written into a file an agent reads first, so a hostile command can still put whatever it likes in `CLAUDE.md`. Landlock rights beyond ABI 1 are applied best-effort on older kernels. The trust model's one promise would then rest on the sandbox being complete on every OS and kernel, which is not a promise this tool can keep. ADR 0007's model stays; the sandbox is about `inputs=`.
- **Allow listing only the directories on the path to each input.** Tighter: an undeclared change to a directory's listing would be caught. Rejected: Landlock's directory-read right is hierarchical, so Linux cannot express it. macOS could, but a region would then fail on one OS and pass on the other. Listing is allowed across the repository on both. The sandbox enforces file contents, not file names.
- **Run unsandboxed with a warning where no sandbox exists.** More regions render. Rejected: the flag's whole meaning is that the declaration was enforced. "Unavailable" is the machine's answer, like an unreadable input, so it is tier 2.
- **Close the Linux network with `unshare(CLONE_NEWUSER | CLONE_NEWNET)`.** Stronger isolation. Rejected: unprivileged user namespaces are disabled or restricted on some distributions, Ubuntu's AppArmor rule among them. A seccomp filter needs no privilege.
- **Run the command in a container.** Complete isolation. Rejected: a container runtime becomes a dependency of `run`, and the image's contents become an input nobody declared.
- **An OS sandbox scoped to `inputs=`, with trust unchanged.** Chosen.

## Consequences

- `sandbox` is a flag, so it is part of the canonical opener: adding or removing it makes the region stale. The snapshot and the format constant are unchanged.
- `sandbox` with `volatile` is a parse error, because there are no inputs to allow. `sandbox` with `workdir=copy` is a parse error too, because a copy holds every file.
- Tools that read from `$HOME` fail inside the sandbox: `~/.gitconfig`, toolchains under `~/.rustup`, binaries under `~/.cargo/bin` that resolve elsewhere. So do tools that need a Mach service on macOS or any socket on Linux, and `PATH` directories inside the repository, such as `node_modules/.bin`.
- The sandbox is applied last, after `doctor`'s environment wrap, so no wrapper can remove it. `doctor` runs sandboxed regions inside it. `trace` runs them outside it, because the reads it looks for are the ones the sandbox refuses.
- The Linux path was verified on aarch64 with Landlock ABI 9 only. x86_64 and older kernels are untested.
