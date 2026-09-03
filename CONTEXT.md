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

**Sink**:
The thing that shapes a loader's text into the form written into the region, such as a fenced block or a table.
_Avoid_: renderer, formatter, view

**Sum**:
A hash stored in a closer. The input sum records what the region was computed from; the output sum records what the tool wrote.
_Avoid_: checksum, digest, hash (as a noun for the stored value)

**Snapshot**:
The content of a region's declared inputs at the moment the loader ran, as the thing the input sum is taken over.
_Avoid_: cache key, fingerprint

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
The paths a region declares it was computed from. The tree loader's inputs are implied by its listing; an exec region declares them or declares itself volatile.
_Avoid_: dependencies, sources, watch list

**Volatile**:
A region that declares it has no inputs worth snapshotting, so it can never be known fresh from the file alone.
_Avoid_: dynamic, uncached, live

**Region root**:
The directory of the template file, which every relative path in a marker is resolved against and in which an exec command runs.
_Avoid_: base dir, cwd, context directory
