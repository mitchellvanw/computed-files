# Computed Markdown

A tool that keeps generated spans of a hand-written text file current by computation. The document is a view; the truth lives in a data source elsewhere.

## Language

**Template**:
The hand-written file that contains markers. The only file a person edits.
_Avoid_: source file, input file, master

**Rendered file**:
The file the tool writes, with every region filled in. What readers and other tools open.
_Avoid_: output, artifact, generated file, build product

**Region**:
The span between an opener and a closer whose body the tool owns and replaces.
_Avoid_: block, section, slot, placeholder

**Marker**:
A comment line that opens or closes a region. The opener names the loader and its attributes; the closer carries the sums.
_Avoid_: tag, directive, annotation

**Name**:
The optional per-file identifier an author gives a region so reports and later tooling can refer to it stably.
_Avoid_: id, key, label

**Loader**:
The thing that produces a region's text and the snapshot of the inputs it read.
_Avoid_: source, data source, provider, generator

**Native loader**:
A loader that reads files, history or a pinned document and runs no command from the repository, so it needs no trust. Every loader except `exec` and `transcript`.
_Avoid_: built-in, safe loader, plugin

**Recipe**:
A named opener in `computed.toml`. A region that writes `use recipe=NAME` stands for it, and its expansion is that region's opener for every purpose: paths, sums, trust.
_Avoid_: preset, macro, alias, template (that is the file with the markers)

**Sink**:
The thing that shapes a loader's text into the form written into the region, such as a fenced block or a table.
_Avoid_: renderer, formatter, view

**Sum**:
A hash stored in a closer. The input sum records what the region was computed from; the output sum records what the tool wrote.
_Avoid_: checksum, digest, hash (as a noun for the stored value)

**Snapshot**:
The content of a region's inputs, or of the part a projection names, at the moment the snapshot step ran, as the thing the input sum is taken over.
_Avoid_: cache key, fingerprint

**Projection**:
The part of a file a region reads, named by `lines=`, `section=`, `anchor=` or `key=`. The snapshot holds only that part, so an edit elsewhere in the file leaves the region fresh. The text a projection yields is its slice.
_Avoid_: range, selector, excerpt, filter

**In-place**:
The layout where the template and the rendered file are the same file.
_Avoid_: rewrite mode, inline mode

**Copy**:
The layout where the template is a separate file and the rendered file is written next to it at the canonical path.
_Avoid_: template mode, build mode, materialised

**Drift**:
Any difference between the rendered file as it is and as the tool would write it now, whether from changed inputs, a changed template, or a hand edit.
_Avoid_: stale (for the file as a whole), dirty, out of date

**Hand edit**:
A change made to a rendered file by a person or an agent rather than by the tool.
_Avoid_: manual edit, external write

**Inputs**:
The paths a region declares it was computed from. A native loader's inputs are implied by its opener: a listing, a `src=`, a glob, the history of a path. An exec or transcript region declares them in `inputs=` or declares itself volatile.
_Avoid_: dependencies, sources, watch list

**Read set**:
The files a region's snapshot actually read, recorded as it is taken. What `run` settles by, and what `affected`, `watch` and the editor hover use to know which regions a file reaches.
_Avoid_: dependencies, inputs (those are declared), trace

**Fresh**:
The state of a region whose input sum and output sum both match what the tool would compute now. The only state `run` leaves untouched.
_Avoid_: up to date, cached, clean (that is a file)

**Stale**:
The state of a region whose input sum no longer matches its snapshot, so its inputs or opener have changed since it was rendered.
_Avoid_: out of date, dirty, invalidated

**Format constant**:
The per-loader integer folded into the input sum, bumped when a loader's rendering of the same inputs changes, so an upgrade re-renders only what it changed.
_Avoid_: schema version, tool version, cache version

**Moved**:
What `doctor` calls a fresh region whose loader now prints something other than its body: the output changed without its inputs. `check` cannot see it; `run --force` catches it up.
_Avoid_: stale (its inputs did not change), nondeterministic (that is two runs disagreeing)

**Volatile**:
A region that declares it has no inputs worth snapshotting, so it can never be known fresh from the file alone.
_Avoid_: dynamic, uncached, live

**Edited**:
The state of a region whose body no longer matches its output sum. One kind of drift; the only one `run` refuses to write over.
_Avoid_: dirty, tampered, modified

**Unrendered**:
The state of a region whose closer carries no sums, so the tool has never written it and renders it without regard to the body.
_Avoid_: new, empty, fresh (that means the opposite)

**Region root**:
The directory of the template file, which every relative path in a marker is resolved against and in which an exec command or a transcript runs.
_Avoid_: base dir, cwd, context directory

**Trusted**:
The state of a repository that a person on this machine has granted permission to run its exec and transcript regions. Recorded outside the repository, per clone.
_Avoid_: allowed, whitelisted, enabled

**Untrusted**:
The state of a repository with no such grant. The only state in which `run` skips an exec or transcript region.
_Avoid_: unsafe, blocked, sandboxed

**Grant**:
The record that makes a repository trusted, written by `computed trust` and removed by `computed untrust`.
_Avoid_: allowlist entry, permission, token

**Sandbox**:
The operating-system confinement an exec or transcript region asks for with the `sandbox` flag: it reads only its declared inputs and the system's programs, writes only its own temporary directory, and reaches no network. It makes `inputs=` enforced. It does not replace trust.
_Avoid_: jail, container, isolation, untrusted (that is a clone)

**Pin**:
The `sha256=` a remote region's opener carries: the SHA-256 of the document it renders. `computed update` moves it. `run` fails the region when the fetched bytes do not match it.
_Avoid_: lock, checksum, version

**Allowlist**:
The url prefixes remote regions may fetch under on this machine, kept in `remote.toml` beside the trust store and never in the working tree. Independent of trust: a grant allows no url, and an allowed url needs no grant. A remote region whose url is not on it is reported `disallowed` and skipped.
_Avoid_: whitelist, grant (that is for commands), trust
