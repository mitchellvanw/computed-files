---
status: accepted
---

# A marker is a comment in the file's own syntax, and discovery reads code only when asked

A marker is written in the comment of the file it sits in. The file's name, else its extension, picks one of six syntaxes from a table compiled into the binary: Markdown and HTML use `<!-- -->`, and the others use `//` (with Rust's `///` and `//!`), `#`, `--`, or a one-line `/* */`. A file with an extension the table does not know reads as Markdown, as every file did before. In a line comment, a line opens a region only when `computed` is followed by a loader name the tool knows, so `// computed values are cached` stays prose. A new sink, `as=comment`, writes the text behind the opener's leader, and outside Markdown a loader that would fence its text writes it raw. The comment joins the input sum's loader line as ` comment="//"`, and only when it is not `<!--`. With no paths, discovery still reads `.md` and `.markdown`. A `[discover]` table in the repository root's `computed.toml` adds extensions and file names.

## Context

Until now the grammar knew one comment. `marker::parse` looked for whole-line `<!-- computed … -->` in any file it was given. Discovery only walked Markdown, but an explicit path was read whatever its extension, and the site page said a region "works today in … a Rust source file". It did not. `<!-- computed tree -->` on a line of Rust is a syntax error, and so it is in Python, YAML, a Makefile or CSS. The only code files that could hold a region were the ones where an HTML comment happens to be legal: HTML, XML and SVG.

The blocks worth computing in code are the same kind the tool computes in Markdown. A crate's `//!` doc lists its modules. A Makefile's help target lists the scripts. A YAML workflow has a matrix of versions, and a CSS file carries a generated palette. Each is copied by hand today, and nothing notices when it goes stale.

Three things were already fixed. Existing sums must not move: every rendered region in every repository would read `stale` on upgrade. The pre-commit hook and CI must discover the same files. And a file the tool writes must parse back to the same regions.

## Considered Options

- **Keep `<!--` and wrap it in the host comment,** as in `// <!-- computed tree -->`. There is one marker shape, and `grep '<!-- computed'` finds every region. Rejected: the parser still needs a leader per syntax to strip, which is the same table. Every opener in a code file would carry two comment syntaxes, and `as=comment` would still need the leader.
- **Guess the comment from the line,** accepting any of `<!--`, `//`, `#` or `--` in any file. No table is needed. Rejected: `# computed` in Markdown is a heading, and `-- computed` in Markdown is prose. The same line would mean different things by accident.
- **Name the syntax in configuration,** mapping extensions to comments in `computed.toml`. A new language would need no release. Rejected: an explicit path would then parse differently depending on the file a directory above it holds, and the opener cannot say its own syntax, because it cannot be found without it. Extensions are how editors and linguist decide, and one line in `EXTENSIONS` adds one.
- **Hold line comments to the Markdown lookalike rule,** where any comment starting `computed` is a marker and an unknown loader is a parse error. One rule for every syntax. Rejected: in Markdown, `<!-- computed` almost never appears by chance. In code, "computed" is an ordinary English word at the start of a comment, and upgrading would turn such comments into parse errors across a codebase. With the known-loader test, a typo in the loader name makes the opener prose, and its closer then fails as a closer without an opener. The mistake is still caught.
- **A sigil for code,** such as `// @computed`. The opener is unambiguous. Rejected: it gives each syntax a second spelling to learn. The known-loader test reaches the same certainty with one spelling.
- **Discover every file with a known syntax by default.** Regions in code work with no setup. Rejected: an upgrade would change which files a bare `computed check` reads in every repository that already uses it. A comment such as `# computed tree is the loader` would become a parse error, and the walk would read every source file. With `extensions = ["rs","sh","toml","yml","html"]`, a `check` of this repository went from 5.2 ms to 8.3 ms.
- **A command-line flag for extra extensions.** Opt-in without a file. Rejected: the hook, CI, `watch` and the editor would each have to be given the same flag, and one that is left out discovers less without saying so. A committed table gives them all one answer.
- **Put the comment in every input sum,** `comment="<!--"` included. One rule. Rejected: it moves every existing sum once. Leaving `<!--` out means a Markdown or HTML sum is what it always was.
- **Put the syntax, not the comment, in the sum.** Rejected: the body depends on the comment, not the syntax. `//` and `//!` in one Rust file give different `as=comment` bodies.
- **Comments in the file's own syntax, known loaders for line comments, `[discover]` for code, and `comment=` only when not `<!--`.** Chosen.

## Consequences

- A region can live in Rust, Go, TypeScript, Python, shell, TOML, YAML, SQL, Lua, CSS, a Makefile or a Dockerfile. `as=comment` puts generated text inside a doc comment; with `lang=`, it fences it first, so `//!` holds a code block.
- No sum moved, so no existing region went stale. The cost: Markdown and HTML both write `<!--`, so a fence-default region moved from `.md` to `.html` keeps its sums and its fenced body. `check` calls it fresh, and only `run --force` rewrites it raw. The spec lists this as open.
- The table is compiled in. A language outside it needs a release, and until then its files read as Markdown when named, so only an HTML-comment marker is found in them.
- `computed.toml` gains a second table. [ADR 0022](0022-recipes-in-computed-toml.md) said the file holds recipes and nothing else, "so the file cannot grow a second discovery … mechanism unnoticed". `[discover]` is that mechanism, grown in the open: a named table, validated against the syntax table, with an unknown extension or key as exit 2. Discovery reads only that table, so a broken recipe does not stop a walk.
- Only Markdown has fences, so the parse-back check runs per syntax. Outside Markdown, text holding a marker line is a loader failure under any sink. In HTML, a `<script>` or `<style>` element holds no markers. Its text is not HTML, so `<!--` there opens no comment. Without the rule, this repository's `prototypes/*.html` do not parse.
- `toc` and `dupes` stay Markdown only. `toc` in another syntax is a region error. `merge --install` still adds only `*.md` and `*.markdown` to `.gitattributes`, and a code file with regions needs its own line.
- Every consumer now carries a syntax: `strip_sums(path, bytes)`, the guard and the language server by path, the merge driver by `%P`. `marker::parse` became the Markdown case of `parse_as`.
