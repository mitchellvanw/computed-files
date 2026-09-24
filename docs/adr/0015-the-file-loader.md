---
status: accepted
---

# The `file` loader

A third loader, `file`, includes one file verbatim: `<!-- computed file src=docs/shared.md -->`. Its snapshot is the one `inputs=src` would take, and its text is the file's content, both with closer sums taken out ([ADR 0014](0014-snapshots-ignore-sums-and-run-settles-across-files.md)). It runs no command, so it needs no trust and renders in an untrusted clone. The default sink is `raw`; `as=fence lang=…` shows a file as code.

## Context

The spec kept `file` "in the fog", wanted only if an untrusted repository needed includes. The audit found two needs it answers that `exec` answers badly:

- **Shared sections.** `CLAUDE.md`, `AGENTS.md` and `.github/copilot-instructions.md` often carry the same paragraphs. `exec cmd="cat shared.md" inputs=shared.md` works, but it is untrusted on a fresh clone, so the hook fails until someone runs `computed trust` for what is only a copy.
- **Marker examples.** Showing a template, markers included, inside a fence is how this repository documents itself. Once loader text may hold marker lines inside a fence, an include of such a file is exactly what `file src=… as=fence` expresses.

## Considered Options

- **Keep using `exec cat`.** No new grammar. Rejected: it needs trust for something that runs no code, and it spells out the input twice.
- **A `file` flag on `exec`.** Rejected: it would be a loader hiding inside another loader's attribute set, with its own rules for trust and snapshots.
- **A third loader.** Chosen. The loader set is a closed enum, and this is the variant the spec already named.

## Consequences

- The grammar gains `file` with one attribute, `src=`, required. It resolves against the region root and must stay inside the repository, like `tree src=`. It must name a file, and not the template itself.
- A missing or escaping `src=` is a hard error for the region ([ADR 0013](0013-a-region-the-tool-cannot-answer-skips-only-itself.md)). Content that is not UTF-8 is a loader failure.
- Format constant 1.
- `render` still skips only `exec` in an untrusted clone.
