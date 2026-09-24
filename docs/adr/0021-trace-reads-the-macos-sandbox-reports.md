---
status: accepted
---

# `trace` reads the macOS sandbox's own reports

`computed trace` needs the files an exec command opens for reading, as an unprivileged user. On macOS it runs the command under `sandbox-exec` with a profile that allows everything and asks for a report on every read inside the repository:

```
(version 1)(allow default)(allow file-read-data (subpath "<repo root>") (with report))
```

The kernel writes one `Sandbox: … allow file-read-data <path>` line per read to the unified log. `trace` reads them back with `log show`, keeping only the lines between two sentinel reads of fresh temporary files, one made before the command and one after, and polls until the second sentinel appears. On Linux it runs `strace -ff -y` and reads the opens and execs from one log per process. Anywhere else, and wherever the tracer is missing or refused, `trace` exits 2 before touching a file.

## Context

`trace` compares what a command read with what its `inputs=` declares: an undeclared read is a false fresh waiting to happen, and an unused input is a false stale. The comparison is worth nothing unless the reads are complete. And it has to work where people run it, on a developer's Mac without `sudo` and in a CI container.

macOS guards every general-purpose tracing facility behind root, System Integrity Protection or an entitlement. Each one was tried on a macOS 15.6 machine with SIP on.

## Considered Options

- **`fs_usage`.** The standard file-activity tracer. Rejected: "must be run as root".
- **`eslogger`,** the Endpoint Security logger. Rejected: needs superuser, and Endpoint Security itself needs an entitlement.
- **`dtrace` / `dtruss`.** Complete and precise. Rejected: "DTrace requires additional privileges" with SIP on.
- **Interposing `open` through `DYLD_INSERT_LIBRARIES`.** No privilege needed in principle. Rejected: SIP strips the variable for `/bin/sh` and every system binary, so it arrives empty. It would also miss static binaries and direct syscalls.
- **`log stream`** in place of `log show`. Live, with no polling. Rejected: it does not deliver sandbox reports to a non-root user. `log show` does, completely: 1503 of 1503 reads in a 1500-file test.
- **FSEvents.** Unprivileged. Rejected: it reports changes, not reads.
- **`sandbox-exec` in report mode, read back with `log show`.** Chosen. On Linux, `strace` is the unprivileged equivalent; `fanotify` needs `CAP_SYS_ADMIN`, and `LD_PRELOAD` misses static binaries.

## Consequences

- A trace costs about 0.7 s per region on macOS, most of it waiting for the log.
- The sentinels are what make consecutive traces clean. Two traces of the same repository running at the same time can still mix their reads.
- A command that uses `sandbox-exec` itself fails under `trace`. The kernel might throttle reports at very high read rates; none was seen.
- `sandbox-exec` is marked deprecated in its man page, and its report lines are not a documented format. A macOS release could break this backend, and `trace` would then exit 2 on macOS rather than report wrongly.
- On Linux, `strace` must be installed and ptrace permitted. A container without the capability fails the probe, and `trace` exits 2 with what to install. A relative `execve` path is resolved against the region root, so it can be wrong after a `cd`.
- Both log parsers sit behind a `Tracer` trait, and the comparison, the suggestion and `--write` are tested with a fake tracer.
