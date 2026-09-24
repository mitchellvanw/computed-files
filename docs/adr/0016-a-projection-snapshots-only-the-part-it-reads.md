---
status: accepted
---

# A projection snapshots only the part it reads

A region that reads part of a file names the part with a projection: `lines=A-B`, `section=TEXT`, `anchor=NAME` or `key=a.b.c`. It is written on `file src=`, required on `value`, and allowed as a suffix on a literal `inputs=` entry, as in `inputs="Cargo.toml#key=package.version,src/cli.rs#lines=40-90"`. The snapshot entry is `path#<canonical projection>` `\0` length `\0` the projected bytes `\0`, so an edit outside the part leaves the region fresh. A projection that finds nothing is a hard error for its region. `symbol` follows the same rule with its own key, `src#item=…,part=…`.

## Context

Until now every snapshot entry was a whole file. `file` included one whole, and an `inputs=` entry could only name whole files. A region that showed one field of a file went stale whenever any other part of that file changed:

- **A version.** `exec cmd="grep -m1 ^version Cargo.toml" inputs=Cargo.toml` is stale after every dependency bump, and it needs trust to read one line.
- **A shared section.** Including the "Install" section of `README.md` into `CLAUDE.md` meant a script to cut it out, and the region went stale on every README edit.
- **A signature.** Documenting one function meant hashing the whole source file, so a changed function body made the docs stale.

A false stale costs a `run`, and under pre-commit a failed commit. Worse, a region that is stale for no reason teaches people that `stale` means nothing.

## Considered Options

- **Snapshot the whole file and apply the projection only to the text.** The snapshot format stays as it is and only the rendering changes. Rejected: it fixes the render and keeps every false stale above.
- **Leave it to exec,** with `sed -n`, `jq` or a script. Needs no new grammar. Rejected: `inputs=` can only name whole files, so the snapshot stays whole, and every one of these regions needs trust for what is only a read.
- **Clamp or empty a projection that finds nothing.** A range past the end of the file would render what is there, and a missing heading would render nothing. Rejected: a heading renamed away would then render an empty region that `check` calls fresh. The anchor going missing is exactly the change the author has to hear about.
- **Snapshot the projected bytes under a key that names the projection.** Chosen. The key keeps two projections of one file, or one file taken whole and in part, as separate entries in one sorted snapshot.

## Consequences

- An edit outside the part leaves the region fresh. An edit inside it, or a change to the projection in the opener, makes it stale.
- `key=` snapshots the value as canonical compact JSON with table keys sorted. Reordering a TOML file or editing its comments is not a change. A string that becomes a number with the same text is.
- Entries without a projection snapshot byte for byte as before, so no format constant moved and no existing region went stale.
- `#` followed by `lines=`, `section=`, `anchor=` or `key=` now splits an `inputs=` entry. A path that literally contains one of those changes meaning. Section text that contains a comma cannot be used in `inputs=`, which splits on commas.
- A projection on a wildcard path is a parse error: a glob names many files and has no one part. A projection on the template itself is an error. The `toc` loader reads the template's own headings.
- `section=` knows ATX headings only, not setext ones.
- `adopt` can write an edit back into a slice, because `Projection::range` gives its byte span. An anchor whose slice skips nested marker lines is not one span, so `adopt` refuses it.
