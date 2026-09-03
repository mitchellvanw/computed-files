---
status: accepted
---

# Loader text is normalised and exec runs in a pinned environment

The output sum is taken over the body bytes, so anything that changes those bytes for the same inputs counts as drift. Two sources of such change are not in the inputs at all: the shape of the text a command prints, and the environment it prints it under. Both are fixed before a sink sees the text.

Normalisation, applied to every loader's text in the `sink` module before either sink shapes it: CRLF and lone CR become LF; trailing newlines are stripped, and the sink then owns the body's line structure (`raw` wraps in blank lines, `fence` does not); trailing spaces and tabs on a line are preserved, because markdown gives them meaning and padded tables carry them; a C0 control byte other than tab, LF and CR is a loader failure, as is a line that would parse as a marker. The rules are part of what a format constant covers: a change to them bumps both `tree` and `exec`.

Environment for `exec`: the inherited environment with `LC_ALL=C`, `LANGUAGE=` and `TZ=UTC` set unconditionally, plus the three `COMPUTED_*` variables. Nothing else is touched. The `tree` listing and its snapshot share one byte-order walk, directories and files interleaved.

## Considered Options

- **Pass bytes through untouched.** Rejected: a command that prints CRLF on one machine and LF on another produces a body that differs under the sum and looks identical in an editor, and one versus three trailing newlines is not a change anyone wants to see in `check`.
- **Strip trailing whitespace per line as well.** Rejected: two-space hard breaks and padded table cells are content. A tool that emits trailing whitespace does so deterministically, so it does not threaten the sum.
- **Inherit the user's locale and timezone.** Rejected: `sort`, `ls`, `date` and error messages vary by locale and zone, so the developer's `run` and CI's `check` would disagree about the same inputs. A command that wants a locale sets it inside `cmd=`.
- **Pin more of the environment, such as `PATH`.** Rejected: reproducibility of the tool set is the repository's job, and stripping `PATH` breaks the common case.

## Consequences

- The `sink` module holds one normalisation function and both sinks; the test for it is a table of byte sequences.
- A command's own line endings and trailing-newline count are never visible in the rendered file, so a region cannot be used to reproduce them.
- Pinned variables are a closed list in the spec; adding one is a deliberate change with a format-constant bump.
