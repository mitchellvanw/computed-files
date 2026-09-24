---
status: accepted
---

# A region can sit inside a line, in Markdown and HTML

In Markdown and HTML, an opener comment that does not stand alone on its line, with its closer later on the same line, is an inline region. Its body is the text between the two comments:

```markdown
The current release is <!-- computed value src=Cargo.toml key=package.version -->0.2.0<!-- /computed in=… out=… -->.
```

The loader's text must be one line, and only the raw sink applies. In Markdown, inline markers inside a code span or a fenced block are prose. The closer carries both sums in full, the opener never gets the `| do not edit; run computed` suffix, and the input sum's loader line gains ` inline`. Reports give the place as `path:line:column`, and the column counts characters from 1, as rustc counts them.

## Context

A region used to fill whole lines, and the raw sink wraps its text in blank lines. A value in a sentence, such as a version, a count or a default timeout, therefore became a paragraph of its own. The prose had to be rebuilt around it, or the sentence left hand-written.

This repository's site shows the cost. The install line on `docs/index.html` is a `<pre>` block with `# v0.2.0` in it. Before, two `exec` regions ran `scripts/site-install.sh`, which printed the whole `<pre>` with its markup and the version it read from `Cargo.toml`. The markup lived in a shell script, and the regions needed trust to read one TOML key. That is the case [ADR 0017](0017-trust-gates-running-repository-code-and-nothing-else.md) set out to remove. `value` could read the key without trust. It could not put the key in the middle of a line.

Line comments cannot hold an inline region. `//` and `#` run to the end of the line, so nothing written after the opener is outside the comment. `<!-- -->` ends on its line and is invisible when Markdown or HTML renders, so only those two syntaxes can do this.

## Considered Options

- **Keep regions whole-line and generate the sentence.** The region renders the sentence around the value, from an exec command or a template attribute. No grammar change. Rejected: the author's prose moves into an opener or a script, and editing a sentence becomes a render.
- **A short closer, with the sums kept elsewhere.** A footer comment per file, or the sums keyed by name in a sidecar, would keep a sentence short in the source. Rejected: [ADR 0002](0002-two-sum-closer.md) keeps the state in the closer so that a region is self-contained, which is why the merge driver, `file` includes and moving a sentence all work. A footer is a second place to keep in step. Truncating the sums contradicts [ADR 0010](0010-sha-256-sums.md). The cost is about 150 characters of hex in the middle of a sentence in the source. The rendered page hides them.
- **Let an inline region span lines** within a paragraph, closing on a later line. A sentence that wraps could hold a value that does too. Rejected: the body would carry newlines inside a paragraph, and the sink would have to know where the paragraph starts and ends. On one line, the body is simply the text.
- **Join multi-line text into one line.** Rejected: a list read into a sentence would silently become a run-on. A loader failure, "the text is N lines, and a region inside a line holds one", keeps the last good value and says why.
- **Read markers in code spans too.** The rule is simpler: every `<!-- computed` on a line counts. Rejected: documentation shows markers in backticks, this repository's spec and site included, and those would all become regions. Code spans follow CommonMark, including spans that cross lines within a paragraph. Indented code blocks, setext headings and HTML blocks are not tracked, which the spec lists as an approximation.
- **Write the suffix into an inline opener** as into any other. Rejected: `| do not edit; run computed` in a sentence doubles the markup, and the sums in the closer already mark the region as the tool's. One written by hand is accepted and dropped on the next render.
- **Leave placement out of the input sum.** Rejected: the body depends on it. The same opener gives a bare value inline and a value between blank lines on lines of its own. Indentation joined the loader line for the same reason. ` inline` is written only for an inline region, so no existing sum moved.
- **Report a byte offset, or the LSP's UTF-16 column.** Rejected: `path:line:column` with a 1-based character column is what rustc prints and what editors jump to. The language server converts to UTF-16 at its edge.
- **Same line, code spans as prose, raw only, full sums, no suffix, ` inline` in the sum.** Chosen.

## Consequences

- `docs/index.html` shows its version with two `value` regions inside the `<pre>` markup. `scripts/site-install.sh` is gone, and a fresh clone renders the page without trust.
- A region is identified by `(line, column)` everywhere a line alone used to be enough: recipe errors, `trace --write`, `doctor`, the language server and the merge of JSON reports across passes. `why --line N` selects every region on the line. JSON reports gain `"column"`, null for a region on lines of its own.
- A `toc` whose heading holds an inline region reads its own template. The value renders in one pass and changes the heading, and the toc needs the next pass. Settling now allows one pass per file plus one, and ADR 0014 is amended to match.
- Some commands treat an inline region more coarsely than a whole-line one. The guard judges an inline region's line as a whole. Prose edits around the value are allowed, and an edit to the closer alone is reported as a body edit. The merge driver merges inline regions as prose, so both sides changing one is an ordinary conflict. `adopt` refuses them, and `stats` counts their bytes and no lines.
- `strip_sums` removes sums from inline closers too, so a template that reads another still ignores the tool's bookkeeping. A golden report under `tests/fixtures` that holds `<!-- /computed -->` mid-line no longer parses when named. It is never discovered, and it is not a template.
- Release 0.2.0 reads an inline region that does not start its line as prose, so a 0.2.0 pre-commit hook passes `docs/index.html` and leaves its values alone. One at the start of a line is a parse error to 0.2.0, since it reads the line as a whole-line marker.
