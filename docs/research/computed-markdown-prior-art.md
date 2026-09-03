# Computed markdown: prior art and design approaches

Research date: 2026-09-02. Sources are primary (official docs, READMEs, source files, man pages); every claim carries a `[n]` pointing at the Sources list. Paragraphs marked **Analysis** are my own reasoning, not sourced.

## Summary

- "Rewrite the region between two comment markers in place" is a well-trodden pattern: cog (any text file, Python generators, checksummed output) [5][6][7][9], zimbatm/mdsh (markdown, shell commands, `<!-- BEGIN mdsh -->`) [3], markdown-magic (markdown, named transforms incl. `fileTree`) [1], doctoc / markdown-toc / terraform-docs / all-contributors (single-purpose) [11][14][12][21]. None of them ship a watch mode; all are run on demand, from pre-commit, or in CI with a `--check`/`--frozen`/`--verify` flag [6][3][2].
- The closest analogue to "computed regions that stay in the same file" is org-babel: a source block plus a `#+RESULTS:` block that is replaced on re-evaluation, with an optional SHA1 `:cache` keyed on the block and its arguments [33][34].
- The closest analogue to "a file is produced by a script, the site rebuilds when the script changes" is Observable Framework's data loaders: `quakes.json` is produced by `quakes.json.sh`/`.py`/`.js`, output goes to `.observablehq/cache`, and the cache is fresh when its mtime is newer than the loader's [41].
- The user's template → render → symlink layout has a precedent in Nix's `result` link and home-manager's generations [67][66], but no surveyed tool puts the link at a path other tooling treats as source. Git stores the link as a `120000` blob containing the link text, so with a gitignored build dir a clone has a dangling `CLAUDE.md` [70][90]; Claude Code follows the link [71][90], GitHub resolves it only when the target is committed [88], Windows needs privileges to create it [71], and a hardlink breaks on the first atomic rename [90]. Copy mode (render straight to the canonical path via temp + rename) keeps the template model and drops those costs; `@path` imports are a Claude-Code-only alternative to embedding [71].
- Decisions the user has to make: (1) template → rendered output at the canonical path (copy) vs symlink to a gitignored build product vs in-place markers vs `@import` sidecar; (2) where the source expression lives (inline in the marker, a sidecar, or a loader script by naming convention); (3) freshness model (mtime chain like Observable/make, content hash like org/knitr/redo-stamp, or always-run); (4) watcher self-write handling (atomic rename + ignore-own-hash); (5) execution policy for shell in markers (cog/mdsh/Templater all execute arbitrary commands; org prompts by default).

## 1. In-place "marked region" tools

| Tool | Marker syntax (verbatim) | Data source expressed as | In place? | Check mode | Watch | License / last push |
|---|---|---|---|---|---|---|
| cog (Python) | `[[[cog` … `]]]` … `[[[end]]]`; any text may surround the tokens on the line [7] | Python code between the markers; `cog.outl()` emits output [5] | Yes with `-r`; else stdout / `-o` [6] | `--check` (+ `--diff`); `-c` checksums output [6] | No | MIT [5]; 2026-08 [87] |
| zimbatm/mdsh | `` `> < include.md` ``, `` `>$ ./gen-md.py` ``, `` `$ cmd` ``; output wrapped in `<!-- BEGIN mdsh -->` … `<!-- END mdsh -->` [3] | Inline shell command or file path in the marker [3] | Yes (default) [3] | `--frozen`: "Fail if the output is different from the input. Useful for CI." [3] | No | MIT; 2026-07 [87] |
| markdown-magic (JS) | `<!-- docs transformName optionOne='hello' -->` … `<!-- /docs -->`; legacy `<!-- AUTO-GENERATED-CONTENT:START -->` … `<!-- AUTO-GENERATED-CONTENT:END -->` [1] | Named transform + attributes: `CODE src="./path" lines=22-44`, `FILE`, `REMOTE url=`, `TOC`, `fileTree src="./src" maxDepth=2`; custom transforms in `md.config.js` [1] | Yes by default; `output.directory` for elsewhere [1] | `dryRun` option [1] | No | MIT (README); 2026-07 [87] |
| embedme | First line of a fenced code block is a comment naming the file: `// path/to/file.ts#L20-L30` [2] | File path + GitHub line-range syntax [2] | Yes; `--stdout`, `--dry-run` [2] | `--verify` [2] | No | MIT; 2024-10 [87] |
| doctoc | `<!-- START doctoc generated TOC please keep comment here to allow auto update -->` … `<!-- END doctoc generated TOC please keep comment here to allow auto update -->`, or `<!-- START doctoc -->`/`<!-- END doctoc -->` [11] | Implicit (headings of the same file) | Yes; `--update-only` touches only files that already have markers [11] | No | No | MIT; 2026-08 [87] |
| markdown-toc | `<!-- toc -->` … `<!-- tocstop -->` [14] | Implicit | `-i` [14] | No | No | MIT; 2024-08 [87] |
| terraform-docs | `<!-- BEGIN_TF_DOCS -->` … `<!-- END_TF_DOCS -->`; template `{{ .Content }}` between them is mandatory [12] | Implicit (the module in cwd) | `output.mode: inject` (partial) or `replace` (whole file); creates file if missing, appends if markers absent [12] | via pre-commit hook [13] | No | MIT; 2026-08 [87] |
| all-contributors | `<!-- ALL-CONTRIBUTORS-LIST:START - Do not remove or modify this section -->` … `<!-- ALL-CONTRIBUTORS-LIST:END -->` [21] | `.all-contributorsrc` sidecar [21] | Yes | No | No | 2026-09 [87] |
| markdown-magic-directory-tree | `<!-- AUTO-GENERATED-CONTENT:START (DIRTREE:dir=./&depth=1) -->` [19] | options `dir`, `ignore`, `depth`, `onlyDirs` [19] | via markdown-magic | – | No | archived, moved to markdown-magic-plugins [19] |
| markdown-notes-tree | boundary markers around a tree in README.md; "you can add anything below the tree (or move the tree) and the tool will respect the tree's boundaries" [20] | Folder of notes | Yes | – | No | MIT; 2026-05 [87] |
| bashup/mdsh | ```` ```shell ```` and ```` ```shell @mdsh ```` fences [4] | Not a region rewriter: compiles markdown to a bash script; `--out` replaces the *output* file only on success [4] | No (separate output) | – | No | MIT; 2022-07 [87] |

Details worth keeping:

- **cog's checksum.** With `-c`, the end line becomes `[[[end]]] (sum: …)` (older files: `(checksum: …)`); on rerun a hand-edited region fails with "Output has been edited! Delete old checksum to unprotect." [9]. `--check` is implemented as `if old_text != new_text: check_failed = True` [10]. `-x` "Excise all the generated output without running the Pythons"; `-d` deletes the generator code from the output (one-shot template mode); `--markers 'START END END-OUTPUT'` customises the tokens [6]. Cog writes the output file directly with `open(fname, "w")`, no temp-and-rename [10].
- **cog's rationale for in-file output**: "It writes its results back into the original file while retaining the code it executed", so files "can be checked into source control without worrying about keeping the source files separate from the output files, without modifying build procedures" [8]. Multiple chunks in one file share one globals dict [7].
- **mdsh's command grammar** is `[langname] <out_cmd> <in_cmd> [data_line]`: `<` reads a file, `$` runs a command, `> lang` emits a code block, bare `>` emits raw markdown, `!` evals into shell variables. Containers: inline code, fenced block, HTML comment (`<!-- > yml $ ... -->`), or link (`[> < data](filename)`). `--clean` removes all generated blocks; `--work-dir` sets the execution dir (defaults to the input file's dir) [3].
- **markdown-magic's `fileTree` transform** (`<!-- docs fileTree src="./src" maxDepth=2 -->`) is exactly the user's motivating case, already implemented [1].
- **Render-time include (not in place)**: mdBook `{{#include file.rs:2:10}}` and anchor form `{{#include file.rs:component}}` with `ANCHOR:`/`ANCHOR_END:` comments in the source [15]; preprocessors receive `[context, book]` as JSON on stdin and return the mutated book on stdout, so source files are never touched [16]. remark-code-import uses a fence attribute: ```` ```js file=./say-hi.js#L3-L6 ````, refuses paths outside `rootDir` unless `allowImportingFromOutside` [17]. HashiCorp's remark-include-markdown resolves `@include` statements recursively at build time [18]. pandoc filters do the same at the AST level: `~~~~ {include="README"}` is replaced at render time; "A 'filter' is a program that modifies the AST, between the reader and the writer" [26].

**Analysis.** The market splits cleanly into (a) *rewriters* that keep source and output in one file and need a check mode to stay honest in CI, and (b) *render-time includes* that never touch the source and therefore need a renderer in the read path. CLAUDE.md is read raw by Claude Code, so (b) is out for that case; what works is (a), a template rendered to the canonical path (§5), or a Claude-Code-native include (`@path`, §5).

## 2. Template → output generators and computed documents

| Tool | Mechanism | Data source | Writes back into source? | Watch |
|---|---|---|---|---|
| gomplate | Go templates, `-f tmpl -o out`, `--input-dir/--output-dir` [22] | `-d name=URL`; schemes `file`, `http(s)`, `env`, `stdin`, `git`, `s3`, `gs`, `consul`, `vault`, `aws+smp`, `merge`; MIME-typed json/yaml/csv/toml/env/text; `{{ ds "name" }}` [23] | No | None documented [22] |
| j2cli | `j2 template data`, formats ini/env/yaml/json, `-o` [24] | file arg | No | None [24]; repo archived [87] |
| envsubst | `$VARIABLE`/`${VARIABLE}` replaced; SHELL-FORMAT restricts the set [25] | environment | No | No |
| quarto | `{{< include _content.qmd >}}` on its own line, surrounded by blank lines; underscore prefix keeps partials out of `quarto render` [27] | executable cells | No; results go to `_freeze/` ("`freeze: auto` – re-render only when source changes", commit `_freeze`) [28] | `quarto preview`: "Save the file. The document is re-rendered, and the browser preview is updated." [29] |
| knitr / R Markdown | chunk `cache=TRUE`: "knitr will skip the execution of this code chunk if it has been executed before and nothing in the code chunk has changed"; any chunk option change (except `include`) invalidates; external data via `cache.extra = file.mtime('my-precious.csv')` or `tools::md5sum(...)` [30] | R chunks | No (rendered output) | via RStudio / quarto |
| MyST | ```` ```{include} path/to/file.md ```` with `:start-after:`, `:end-before:`, `:lines:`, `:literal:`; `{embed} #label` or `![](#label)` to embed labeled notebook outputs [31][32] | files, notebooks | No | – |
| org-babel | `#+BEGIN_SRC … #+END_SRC` then `#+RESULTS:`; `:results` handling `replace` (default) / `append` / `prepend` / `silent`; types `table`, `list`, `file`, `code`, `drawer`, `raw` [33]; `C-c C-c` evaluates and "Org creates the 'RESULTS' keyword if one is not already there" [34] | code in the block; `#+CALL: name(args)` and inline `call_name(args)` [34] | **Yes, in the same file** | No (editor command) |
| Obsidian Dataview | ```` ```dataview TABLE author, published FROM #poems ```` , inline `= expr`, ```` ```dataviewjs ```` [36] | vault metadata | No: "Dataview is about displaying, not editing … will always leave them untouched" [36] | live in the view |
| Obsidian Templater | `<% tp.file.title %>`, `<%* %>` JS, `<%+ %>` dynamic (re-evaluated on view) [37]; system user functions "execute system commands and retrieve their output" and must be enabled in settings [38] | JS / shell | Yes at insert time (static commands); dynamic ones re-render | – |
| Logseq | `{{query …}}` simple; `#+BEGIN_QUERY {:title … :query [:find (pull ?b [*]) :where …]} #+END_QUERY` Datalog [39][40] | graph DB | No (rendered) | live |

**org-babel** deserves emphasis as the closest model. `:cache yes` compares "the SHA1 hash value" of the block and its arguments, stores it on the results line, and "when hash values match, Org does not evaluate the code block" [34]. Security is opt-in per evaluation: `org-confirm-babel-evaluate` defaults to `t` ("Org prompts the user for confirmation before executing each code block"); it can be a function to whitelist languages; "Each source code block, in terms of risk, is equivalent to an executable file." [35].

**Analysis.** org-babel already has the three ingredients the user wants: source expression adjacent to output, output written back into the same file, and a hash that skips work. What it lacks is a daemon and a way to hash *inputs* other than the block text (it hashes code+args, not the files the code reads), which is exactly what knitr's `cache.extra = file.mtime(...)` bolts on.

## 3. Live document and reactive notebook systems

**Observable Framework data loaders** [41][42][43]:

- Routing by naming convention: "When a file is referenced from JavaScript via `FileAttachment`, if the file does not exist, Framework will look for a file of the same name with a double extension to see if there is a corresponding data loader." Extensions tried in order: `.js` (node), `.ts` (tsx), `.py` (python3), `.R` (Rscript), `.rs` (rust-script), `.go` (go run), `.java`, `.jl` (julia), `.php`, `.sh` (sh), `.exe` (any executable). "The first match is used." [41]
- Protocol: "Data loaders must output to standard output." The captured stdout becomes the file. Archive loaders (`.zip`, `.tar`, `.tar.gz`, `.tgz`) let one loader produce many files [41].
- Cache: output is saved "to a cache which lives in `.observablehq/cache` within the source root". Freshness rule: "Framework considers the cache 'fresh' if the modification time of the cached output is newer than the modification time of the corresponding data loader source." Touching the loader invalidates; in preview "the preview server will detect that the data loader was modified and automatically run it, pushing the new data down to the client and re-evaluating any referencing code — no reload required!" [41]
- Extensibility: the `interpreters` config maps extension → command, default `{".js": ["node", "--no-warnings=ExperimentalWarning"], ".py": ["python3"], ".sh": ["sh"], …, ".exe": []}` [43].
- Watching: "Live preview applies to Markdown pages, imported JavaScript modules (so-called hot module replacement), data loaders, page loaders, and file attachments … implemented by the preview server watching files and pushing changes to the browser over a socket." [42]

**marimo**: static analysis builds a DAG of cells; "When a cell is run, marimo automatically runs all other cells that reference any of the global variables it defines"; runtime can be `lazy`, marking cells stale instead of running [44]. `marimo edit --watch` streams external edits of the notebook file into the editor; by default "synced code will not be executed automatically, with cells marked as stale instead", `watcher_on_save = "autorun"` flips that [45]. `mo.watch.file(path)` "will trigger dependent cells to re-evaluate when the file is changed"; `mo.watch.directory` reacts only to structure changes, "does NOT react to file content changes", and "will not follow symlinks" [46].

**nbconvert**: `--execute` runs the notebook and "save[s] the result in `mynotebook.nbconvert.ipynb`"; `--inplace` "will overwrite the input file instead of writing a new file" [47]. This is a write-outputs-back-into-source model, but for `.ipynb` JSON, not markdown.

**Analysis.** Observable's design decouples three things the user's sketch conflates: the *reference* (a filename in the document), the *producer* (a script chosen by naming convention), and the *freshness rule* (mtime of loader vs cached output). It never rewrites the markdown; the markdown just names a file. That is the cleanest model for a "separate output" architecture. marimo's `mo.watch.file` is the reactive equivalent of a data dependency edge.

## 4. Watcher and build-system primitives

| Tool | Backend | Debounce / settle | Self-write / atomic-write handling |
|---|---|---|---|
| watchexec | notify | `--debounce` default 50 ms: "Time to wait for new events before taking action" [49] | Respects `.gitignore`/`.ignore`; `-i` glob ignores; `--on-busy-update` `queue`/`do-nothing`/`restart`/`signal`; changed paths in `$WATCHEXEC_WRITTEN_PATH` etc. or via `--emit-events-to json-stdio` [48][49] |
| entr | kqueue/inotify | – | Takes the file list from stdin; `-d` tracks the parent directory so new files are noticed; `-r` restarts a persistent child [50] |
| fswatch | fsevents, kqueue, inotify, ReadDirectoryChangesW, poll | `-l` latency in seconds; `-o` one message per batch [51] | `--event` type filter, `-x` prints flags (Created/Updated/Removed/Renamed) [51] |
| chokidar | fs.watch / polling | `awaitWriteFinish.stabilityThreshold` 2000 ms, `pollInterval` 100 ms [52] | `atomic`: "if a file is re-added within 100 ms of being deleted, Chokidar emits a `change` event rather than `unlink` then `add`"; "filters out artifacts that occur when using editors that use atomic writes" [52] |
| notify (Rust) | inotify, kqueue, FSEvents, ReadDirectoryChangesW, PollWatcher [53] | `notify-debouncer-mini` / `notify-debouncer-full` [53] | Caveat: "the actual events can differ a lot between file editors. Some truncate the file on save, some create a new one and replace the old one." [53]; CC0-1.0 [Cargo.toml] |
| watchman | inotify/FSEvents daemon | `settle` "controls how long the filesystem should be idle before dispatching triggers. The default value is 20 milliseconds." [55] | "Watchman will only run a single instance of the trigger process at a time. That avoids fork-bomb style behavior in cases where your trigger also modifies files." [54]; `since` queries with clock ids `c:123:234` or named cursors `n:whatever` [56] |

Kernel-level facts that shape the design:

- inotify watches are inode-based: `IN_DONT_FOLLOW` means "Don't dereference pathname if it is a symbolic link" (default follows); `IN_MOVE_SELF`/`IN_DELETE_SELF` fire when the watched object itself moves or goes away; `IN_CLOSE_WRITE` fires "when a file opened for writing was closed"; remote and pseudo filesystems get no events [57].
- `rename(2)`: "If newpath already exists, it will be atomically replaced, so that there is no point at which another process attempting to access newpath will find it missing." [58]

Build tools and how they decide "stale":

- make: recipe runs when the target is older than any prerequisite (mtime) [59].
- just: "just is a command runner, not a build system" — no file timestamps, no watch [60].
- redo: `.do` scripts receive `$3`: "the name of a temporary file that will be renamed to the target filename atomically if your .do file returns a zero (success) exit code"; "never modify the file named by `$1` … Only ever write to the file named by `$3`"; `redo-stamp` makes a target count as changed only if its *contents* differ [61][62].
- tup: rules `: hello.c |> gcc hello.c -o hello |> hello`; "tup instruments all commands that it executes in order to determine what files were actually read from (the inputs) and written to (the outputs)" [63].
- ninja: mtime graph; `restat` "causes Ninja to re-stat the command's outputs after execution of the command. Each output whose modification time the command did not change will be treated as though it had never needed to be built" [64].

**Analysis.** Three patterns cover the feedback-loop problem: (1) *redo's* write-to-temp-then-`rename(2)` gives readers a consistent file and lets you compare content before replacing (skip the write if identical, which also keeps mtime stable, the `restat`/`redo-stamp` idea); (2) *watchman's* single-instance trigger plus settle window bounds re-entrancy; (3) *chokidar's* `atomic` and *notify's* caveat tell you that editors themselves do rename-replace, so a path-based watcher on the *directory* is needed, not an inode watch on the file. A daemon that writes what it watches should additionally remember the content hash it just wrote and drop the event if the file still matches it.

## 5. Symlink strategy: template → render → symlinked canonical path

The user's layout, stated precisely: `CLAUDE.md.tmpl` (hand-edited, committed) → render process → `<build-dir>/CLAUDE.md` (materialised output) → `CLAUDE.md` is a symlink to that output. The template is never rewritten. This section evaluates that layout against what symlink-based tools do and against what git, GitHub, Claude Code and editors do with links.

### What the symlink-using tools actually do

- **Nix** is the nearest precedent: `nix-build` "places a symlink to the result in the current directory. The symlink is called `result`" (`-o`/`--out-link`); the link "is automatically registered as a root of the Nix garbage collector" and "disappears automatically when the `result` symlink is deleted or renamed" [67]. The target is an immutable store path; `result` is conventionally gitignored.
- **home-manager**: activation removes links of the old generation then "Symlink files from the new generation into $HOME"; collisions error with "Conflicting managed target files"; `force` = "Paths that should be forcibly overwritten by Home Manager. Caveat emptor!"; `mkOutOfStoreSymlink ./bar` is the escape hatch for files you still want to hand-edit [66].
- **Stow**: "a symlink farm manager"; tree folding ("a single symlink that points to an entire subtree"); two-phase conflict scan, "reports them and terminates without modifying the filesystem" [65].
- **Bazel**: `bazel-bin`, `bazel-out`, `bazel-<workspace>` "are only for the user's convenience, as Bazel itself does not use them", created "only if the workspace root is writable" [68].

Common shape: the link is the *user-facing handle*, the target is *immutable per generation*, a new generation is published by re-pointing the link, and the link itself is not the unit of version control. That matches the user's model well, with one difference: none of these tools put the link at a path that other tooling treats as canonical source (a README, a CLAUDE.md).

### Where the rendered file lives

| Option | Git behaviour | Who can read `CLAUDE.md` |
|---|---|---|
| Rendered file in a gitignored build dir (`.computed/`) | Symlink committed as a `120000` blob whose content is the link text `.computed/CLAUDE.md` [70][90]; a fresh clone has a **dangling** `CLAUDE.md` until the renderer runs [90] | Only machines that have run the renderer; CI, GitHub, and collaborators without the daemon see nothing |
| Rendered file committed alongside the template | Content stored twice (template + output); output diffs appear in every PR; symlink still a 20-byte blob | Everyone, but the symlink now adds no information: the committed output could be `CLAUDE.md` itself |
| Rendered file outside the repo (`~/.cache/...`) | Same as gitignored, plus an absolute link target that differs per machine | Only that machine; `git status` shows the link changing if the target path changes |

`core.symlinks=false` (probed at clone/init time, e.g. FAT/Windows) checks the symlink out "as small plain files that contain the link text" [69], so on such machines `CLAUDE.md` literally contains the string `.computed/CLAUDE.md`. Creating symlinks on Windows "requires Administrator privileges or Developer Mode" [71].

### Do readers follow the link?

- **Claude Code**: yes. The docs endorse `ln -s AGENTS.md CLAUDE.md` and say "In your next session, run `/context` and confirm `CLAUDE.md` appears under Memory files" [71]; the Read tool returned the target's content in a direct test [90]. Two caveats from the same page: Cowork desktop sessions skip a `~/.claude/CLAUDE.md` "that is itself a symlink or hard link" (user scope only, not project scope), and path-scoped rules only match through symlinked paths since v2.1.198 [71]. Subdirectory CLAUDE.md files load lazily "when Claude reads files in those subdirectories" and the project-root file is re-read after `/compact` [71], so a link re-pointed mid-session is picked up at those moments, not instantly.
- **GitHub**: the REST contents API states "If the content is a symlink and the symlink's target is a normal file in the repository, then the API responds with the content of the file. Otherwise, the API responds with an object describing the symlink itself" [88]. So a link to a committed target resolves; a link to a gitignored target is shown as a symlink object (in the web UI, as the link text). GitHub Pages additionally stopped building "sites that contain symbolic links … outside of GitHub Actions" [89], a signal that symlinks in repos get special-cased by platforms.
- **Kernel watchers**: inotify follows the link when adding a watch unless `IN_DONT_FOLLOW` is set, and watches are inode-based [57]. A watch placed on `CLAUDE.md` therefore attaches to the *current* rendered inode; after the renderer publishes a new file by `rename(2)` that inode is orphaned and the watch goes silent. Watch the build directory instead. marimo's `mo.watch.directory` explicitly "will not follow symlinks" [46], one example of a consumer that will not see through the link.
- **Editors**: no primary source found in this pass on VS Code's watcher and symlinks; treat as unverified. (Analysis: editors open the target through the link, but "save" semantics differ: an editor that writes via rename-replace, as notify's docs describe [53], will replace the *symlink* with a regular file, silently detaching it from the build dir. Making the rendered file read-only, as Nix store paths are, turns that into a visible error.)

### Alternatives to "canonical path is a symlink"

| Variant | Mechanism | Assessment (Analysis unless cited) |
|---|---|---|
| Symlink the other way: canonical `CLAUDE.md` is the real rendered file; `.computed/CLAUDE.md` links to it | Renderer writes the canonical path via temp + `rename(2)` [58] | The reverse link is pointless; this is just "template → output at the canonical path" (row below). |
| Copy: renderer writes rendered content to the canonical path (temp + rename) | Same as gomplate `-o`, j2cli `-o`, Style Dictionary, mermaid-cli markdown mode [22][24][82][72] | Canonical file is a regular committed file: git, GitHub, Windows, CI and editors all work; diffs show the generated content; the "is this generated?" signal must come from a banner comment (Style Dictionary `fileHeader` [83]) or a `--check` in CI (cog/mdsh/embedme [6][3][2]). Hand edits to the output are lost on next render, which is the same as the symlink model. |
| Hardlink | `ln .computed/CLAUDE.md CLAUDE.md` | Breaks on the first atomic publish: `rename(2)` replaces the directory entry, so the hardlink keeps pointing at the old inode (verified: after rename-replace the symlink read `# v2`, the hardlink still read `# rendered`) [90]. Git does not represent hardlinks at all. Only viable if the renderer truncates and rewrites in place, which forfeits atomicity. |
| Symlink, target committed | Commit `.computed/CLAUDE.md` too | Works everywhere GitHub-side [88], but the repo now carries template, output and link; the link is decorative. |
| Symlink, target gitignored (the user's default) | `.computed/` in `.gitignore` | Clean repo, but dangling on clone [90]; every consumer that is not this machine must run the renderer first (a `post-checkout`/`post-merge` hook or `pre-commit` [86] can do that). Acceptable if CLAUDE.md is considered a local build product, wrong if collaborators or CI rely on it. |
| `@import` sidecar (Claude-Code-specific) | `CLAUDE.md` is hand-written and contains `@.computed/tree.md` [71] | No link, no rewrite of the hand-edited file, per-region granularity; but only Claude Code understands `@path`, imports load at launch (four-hop limit) and GitHub shows the literal `@.computed/tree.md`. |
| In-place markers (§1) | Rewrite between comments in the canonical file | One file, portable, git-visible, markers invisible to renderers and stripped by Claude Code [71]; costs a self-write loop (§4, §8) and executable text living in a shared file. |

### Comparison with in-place marker rewriting

| Concern | Template → render → symlink | In-place markers |
|---|---|---|
| Hand edits | Only in the template; edits to the canonical path are edits to a build product (or, if the editor rename-replaces, silently detach the link) | Anywhere outside regions survive; inside regions overwritten (cog can detect and refuse via checksum [9]) |
| Watcher feedback loop | None: watch template + inputs, write the build dir, re-point nothing (target path is stable) | Must ignore own writes; temp + rename; content-compare (§4) |
| Atomic publish | `rename(2)` of the rendered file, or re-point the link to `CLAUDE.md.<gen>` Nix-style [67][58] | `rename(2)` of the whole file [58] |
| Git | Link blob + template; output invisible unless committed [70][90] | Output visible in every diff |
| GitHub / other readers | Resolve only if target committed [88]; dangling otherwise | Always render; markers hidden |
| Windows | Symlink creation needs privileges [71]; `core.symlinks=false` yields a text file [69] | No issue |
| Claude Code | Follows the link [71][90] | Reads the file; comments stripped [71] |
| Discoverability | The `.tmpl` next to the link explains itself | The marker explains itself |

**Recommendation (Analysis).** The template → render model is sound and is what gomplate, j2cli, quarto, mermaid-cli and Style Dictionary all do; the *symlink* is the weak link. It buys a stable handle for generation swapping, but the same guarantee comes from writing the canonical path with `rename(2)`, and the link costs a dangling file on clone, a text file on Windows, a detach-on-save hazard in editors, and inode-watch pitfalls. Two workable configurations:

1. **Copy mode (default)**: `CLAUDE.md.tmpl` → `CLAUDE.md` written atomically at the canonical path; output committed; first line of the output is a generated-file banner; `check` mode in CI/pre-commit. This is the user's model minus the link and is what the rest of the ecosystem does.
2. **Symlink mode (opt-in, per file)**: for outputs that are genuinely local build products (large trees, secrets, per-machine data), keep `.computed/` gitignored, commit the link, make rendered files read-only, and provide a `post-checkout` hook so clones are not dangling. Document that GitHub will show the link text.

Either way the template grammar can be the same marker grammar as §1 (a region is `<!-- computed … --> … <!-- /computed -->` in the template; in copy mode the output keeps the markers with the hash, in symlink mode it may strip them), so one parser serves in-place, template and sidecar layouts.

## 6. Renderer ecosystem

| Renderer | Input → output | Emits markdown table natively? | Notes |
|---|---|---|---|
| sqlite3 CLI | SQL → `.mode markdown` / `-markdown` flag [75] | Yes | modes: ascii, box, column, csv, html, insert, json, line, list, markdown, quote, table, tabs, tcl |
| duckdb CLI | SQL/CSV/Parquet → `.mode markdown` ("Markdown table format") [76] | Yes | modes incl. `duckbox`, `jsonlines`, `latex`, `markdown`; querying CSV/JSON files directly makes it a one-stop table renderer |
| csvlook (csvkit) | CSV → "a Markdown-compatible, fixed-width format"; `--max-rows`, `--max-columns`, `--max-column-width` [73] | Yes | Python |
| qsv | `qsv table` aligns with elastic tabstops (not markdown); `qsv sqlp` runs Polars SQL over CSV/Parquet/JSONL → csv/json/parquet/arrow [74] | No | Rust, fast; pipe into duckdb/sqlite for markdown |
| jq | JSON → text; `@csv`, `@tsv`, `@sh`, `@json`; `-r` raw output [81] | No (compose with `@tsv`) | |
| tree | dir → text; `-J` "Outputs the directory tree as a JSON formatted array", `-X`, `-H`, `-L level`, `-I pattern`, `--gitignore`, `--noreport`, `-d`, `--fromfile` [77] | Plain text (fence it) | `--gitignore` is the important flag for the CLAUDE.md case |
| mermaid-cli (`mmdc`) | `.mmd` → svg/png/pdf; **markdown mode**: `mmdc -i readme.template.md -o readme.md` replaces each ```` ```mermaid ```` block with an image reference, files named `readme-1.svg`, `readme-2.svg`, … [72] | n/a | Headless Chromium; note it is itself a template→output tool |
| d2 | `d2 in.d2 out.svg`; `d2 --watch in.d2 out.svg` opens a browser that "live-reload[s] on changes to `in.d2`"; svg/png/pdf [78] | n/a | |
| graphviz | `dot -Tsvg -o out.svg`; `-Tformat[:renderer[:formatter]]` [79] | n/a | |
| vega-lite | `vl2svg spec.vl.json`, `vl2png`, `vl2pdf`, `vl2vg`; stdin accepted [80] | n/a | node |

**Analysis.** sqlite3 and duckdb are the only "give me a markdown table" primitives that also *are* the query engine, which collapses "data source" and "renderer" into one process for the CSV/JSON/SQL cases. For diagrams, the two sink modes the user listed already exist as separate outputs of the same DSL: keep the mermaid text in the region (GitHub renders it) or run `mmdc` and embed an image reference.

## 7. Generalization beyond markdown

- **cog** is format-agnostic by construction: "If the line contains the special character sequence, the whole line is taken as a marker", so `//[[[cog`, `/* cog starts now: [[[cog */`, `--[[[cog` all work, and a shared prefix on every Python line is stripped before execution (the SQL example) [7]. It also supports `--markers` to change the tokens entirely [6].
- **Style Dictionary** is the CSS-tokens instance of template→output: tokens → transforms → formats → platforms, with `fileHeader` hooks that prepend "Do not edit" banners to generated files [82][83]. No watch mode documented [82].
- **envsubst** and **gomplate** are format-agnostic template→output tools; gomplate's datasource layer (json/yaml/csv/toml/env/text by MIME type) is a reusable model for typed inputs [23][25].
- **rehype** "is an ecosystem of plugins that work with HTML as structured data, specifically ASTs" inside unified, the HTML counterpart of remark [84]: same render-time-include pattern is available for HTML.
- **nbconvert** writes computed outputs back into JSON (`.ipynb`) [47], proof that in-place rewriting works for a format with no comment syntax when the format itself reserves output slots.

Comment syntax per target format and what it implies for markers (Analysis, with the syntax facts being standard):

| Format | Comment syntax | Marker feasibility |
|---|---|---|
| Markdown / HTML | `<!-- -->` | Natural; invisible when rendered; stripped from CLAUDE.md by Claude Code [71] |
| CSS / SCSS / JS / C-family | `/* */`, `//` | Natural (cog style) |
| YAML / TOML / shell / Python / SQL | `#`, `--` | Natural; per-line prefix stripping as in cog [7] |
| JSON | none | Needs a slot convention (a key such as `"_computed": {...}`) or template→output; nbconvert shows the slot approach [47] |
| CSV | none | Template→output only, or whole-file generation; a marker would corrupt the data |
| `.ipynb` | none, but cell `outputs` are reserved slots | nbconvert `--execute --inplace` [47] |

Conclusion: a marker grammar must be *comment-syntax-parameterised* (cog's approach: match the token anywhere in the line, remember the line's prefix/suffix) and the tool must fall back to whole-file generation for JSON/CSV.

## 8. Design considerations

**Marker syntax and formatter survival.** All markdown tools use HTML comments, which GitHub and CommonMark do not render, and which Claude Code strips [71]. Prettier's range ignore exists specifically for this case: `<!-- prettier-ignore-start -->` … `<!-- prettier-ignore-end -->`, "designed specifically for auto-generated content from tools like `all-contributors` and `markdown-toc`", top level only, with a required blank line before each marker [85]. The tool should emit those (or accept them adjacent) so formatters do not reflow generated tables. doctoc's long-form marker embeds the instruction "please keep comment here to allow auto update" [11]; all-contributors embeds "Do not remove or modify this section" [21]; both are cheap insurance against humans and LLM agents deleting markers.

**Idempotency and check mode.** Every rewriter offers a no-diff assertion: cog `--check`/`--diff` [6], mdsh `--frozen` [3], embedme `--verify` [2], markdown-magic `dryRun` [1]. cog additionally *protects* the output with a checksum so hand edits are detected rather than overwritten [9]. Generated regions should therefore be deterministic (sorted tree output, no timestamps unless requested), and the tool should have `--check` from day one because that is how it will run in CI and pre-commit.

**Feedback loop when writing the watched file.** Sources give four mechanisms, combine them: write temp then `rename(2)` [58][61]; compare new content to old and skip the write when equal (`redo-stamp`, ninja `restat`) [61][64]; single-flight execution with a settle window (watchman) [54][55]; watch the directory rather than the inode because editors rename-replace (chokidar `atomic`, notify caveat) [52][53]. Add: record the hash of what you wrote and ignore the next event whose content hashes the same. Debounce defaults in the wild are 20 ms (watchman) to 50 ms (watchexec) for event coalescing, with chokidar's 2 s `stabilityThreshold` for "writer finished" [55][49][52].

**Freshness and caching.** Three models in the sources: mtime chain (make, ninja, Observable: cache fresh iff cached-output mtime > loader mtime) [59][64][41]; content hash of the *generator* (org-babel SHA1 of block+args) [34]; content hash of *inputs* (knitr `cache.extra = tools::md5sum(...)`, redo-stamp) [30][61]. Only the last is correct for "folder structure changed" because the generator text and the template file do not change when a new file appears. Practical rule: hash (generator text + declared inputs' content or `tree -J` output) and store it in the end marker like cog's `(sum: …)` [9] so the file itself records what it was computed from.

**Template vs output separation.** cog's argument for one file: nothing to keep in sync, no build changes, checked-in output [8]. Observable's argument for separation: the document only *names* a file, producers are pluggable by extension, cache lives outside the source tree [41]. terraform-docs supports both in one tool (`inject` vs `replace`) [12]. mdsh's `--clean` [3] and cog's `-x` [6] show the reverse operation (strip generated content) is expected too.

**Error handling.** bashup/mdsh's `--out` "will replace filename's contents … if and only if the compilation or run succeeds without any errors" [4]; redo renames `$3` into place only on exit 0 [61]; Observable caches only "when a data loader runs successfully" [41]. Same rule applies here: a failing renderer must leave the previous region intact and report on stderr (or write a one-line `<!-- computed: error … -->` inside the region, which is Analysis).

**Where the source expression lives.** Inline in the marker: markdown-magic attributes, mdsh command, cog Python, org header args [1][3][7][33]. Sidecar config: all-contributors `.all-contributorsrc`, terraform-docs config, markdown-magic `md.config.js` [21][12][1]. Naming convention: Observable `name.ext.lang` [41]. Inline is the most legible in a README; a sidecar is required once expressions get longer than a line or need secrets; the naming convention is best when the output is a whole file.

**Security.** cog runs arbitrary Python from the file [5]; mdsh runs any `$ cmd` in the file with `--work-dir` defaulting to the file's directory [3]; Templater's system commands are off until enabled in settings [38]; remark-code-import refuses to read outside `rootDir` by default [17]; org prompts per block by default and treats each block as "equivalent to an executable file" [35]. A daemon that auto-executes markers from a cloned repo is a supply-chain risk: require a per-repo allowlist or trust prompt (org model), constrain reads to the repo (remark-code-import model), and prefer declarative sources (`tree`, `sqlite`, `csv`) over `sh` where possible.

**Triggers other than a watcher.** pre-commit: a hook that modifies files fails the commit ("Files were modified by this hook"), and the user re-stages; `pass_filenames`, `always_run`, `files` regex control invocation [86]. terraform-docs ships a hook (`id: terraform-docs-go`) and a plain `.git/hooks/pre-commit` example that regenerates and `git add`s the README [13]. doctoc's `--update-only` exists for `lint-staged` [11]. CI with `--check` is the other half. The daemon is a convenience layer on top of these, not a replacement.

## Design decisions and recommendations

| User component | Closest prior art | Recommendation (Analysis) |
|---|---|---|
| Data source | gomplate datasources (typed by MIME) [23]; Observable loaders (any executable, stdout) [41]; markdown-magic transforms (`fileTree`, `CODE`, `REMOTE`) [1] | Two tiers: declarative built-ins (`tree`, `file`, `csv`, `json`, `sql` via sqlite/duckdb) that are safe by default, plus `sh`/loader scripts behind an explicit trust switch. Model the built-ins as "loader emits text on stdout" so custom loaders are the same shape (Observable). |
| Renderer / sink | sqlite3/duckdb `.mode markdown` [75][76]; csvlook [73]; `mmdc` markdown mode [72]; d2 `--watch` [78] | Sinks are text transforms on loader output: `table` (markdown), `fence lang=…` (code block), `raw`, `image` (run mmdc/d2/dot, write the asset next to the file, emit `![]()`). Keep renderer selection in the marker, not the loader. |
| Marker syntax | cog `[[[cog … ]]] … [[[end]]] (sum: …)` [7][9]; markdown-magic `<!-- docs X attrs --> … <!-- /docs -->` [1]; mdsh `<!-- BEGIN mdsh -->` [3] | HTML-comment pair with a namespaced word, attributes in the opener, hash in the closer, e.g. `<!-- computed tree src=. depth=2 --> … <!-- /computed sum=… -->`. Match the token anywhere on the line and preserve the line's prefix/suffix so the same parser works for `/* */` and `#` files (cog). Emit prettier range-ignore comments around tables. |
| Watcher | watchexec (debounce 50 ms, gitignore, on-busy-update) [49]; watchman (settle, single-instance trigger) [54][55]; notify + debouncer for a Rust daemon [53] | Watch the directory containing the file *and* every declared input; debounce ~50 ms; single-flight per file; write temp + rename; skip the write if content is unchanged; remember own-write hash. Dual trigger (template vs data) needs no special casing if both are just paths in the dependency set; only the origin of the change differs. |
| Symlink strategy | Nix `result` [67], home-manager `mkOutOfStoreSymlink` [66], Stow [65], Bazel convenience links [68]; Claude Code follows `CLAUDE.md` symlinks [71][90]; GitHub resolves links only to committed targets [88] | Keep template → render; default to writing the rendered file *at* the canonical path via temp + `rename(2)` (copy mode, committed output, banner + `check`). Offer the symlink layout as an opt-in for gitignored local build products, with read-only targets and a `post-checkout` hook so clones are not dangling. Never hardlink (breaks on rename-replace [90]). |
| Freshness | Observable mtime rule [41]; org SHA1 [34]; knitr `cache.extra` [30]; redo-stamp [61] | Hash the generator expression plus the *content* of declared inputs (for `tree`, hash the `tree -J` output); store in the closer; recompute when the hash differs or `--force`. |
| Trigger surface | cog `--check`, mdsh `--frozen`, embedme `--verify` [6][3][2]; pre-commit [86]; terraform-docs hook [13] | Ship `run`, `check` (non-zero on diff, print diff), `clean` (strip regions, cog `-x`), `watch`. Make `check` the CI/pre-commit entry point; the daemon is optional. |

Open questions the sources do not settle: whether to allow markers to nest (none of the surveyed tools do; cog shares globals across chunks but chunks are sequential [7]); how to reference other computed regions as inputs (marimo's DAG is the model [44]); and whether hand-edited output should be an error (cog) or silently overwritten (everyone else).

## Sources

1. markdown-magic README — https://github.com/DavidWells/markdown-magic/blob/master/README.md
2. embedme README — https://github.com/zakhenry/embedme/blob/master/README.md
3. zimbatm/mdsh README — https://github.com/zimbatm/mdsh/blob/main/README.md
4. bashup/mdsh README — https://github.com/bashup/mdsh/blob/master/README.md
5. cog documentation index — https://cog.readthedocs.io/en/latest/
6. cog, Running cog — https://cog.readthedocs.io/en/latest/running.html
7. cog, Writing the source files — https://github.com/nedbat/cog/blob/master/docs/source.rst
8. cog, Design — https://cog.readthedocs.io/en/latest/design.html
9. cog source, hashhandler.py — https://github.com/nedbat/cog/blob/master/cogapp/hashhandler.py
10. cog source, cogapp.py — https://github.com/nedbat/cog/blob/master/cogapp/cogapp.py
11. doctoc README — https://github.com/thlorenz/doctoc/blob/master/README.md
12. terraform-docs, output configuration — https://terraform-docs.io/user-guide/configuration/output/
13. terraform-docs, pre-commit hooks — https://terraform-docs.io/how-to/pre-commit-hooks/
14. markdown-toc README — https://github.com/jonschlinkert/markdown-toc/blob/master/README.md
15. mdBook, Markdown format (include) — https://rust-lang.github.io/mdBook/format/mdbook.html
16. mdBook, Preprocessors — https://rust-lang.github.io/mdBook/for_developers/preprocessors.html
17. remark-code-import README — https://github.com/kevin940726/remark-code-import/blob/master/README.md
18. HashiCorp remark-plugins, include-markdown — https://github.com/hashicorp/remark-plugins/blob/main/plugins/include-markdown/README.md
19. markdown-magic-directory-tree README — https://github.com/camacho/markdown-magic-directory-tree/blob/master/README.md
20. markdown-notes-tree README — https://github.com/mistermicheels/markdown-notes-tree/blob/master/README.md
21. all-contributors CLI usage — https://allcontributors.org/docs/en/cli/usage
22. gomplate usage — https://docs.gomplate.ca/usage/
23. gomplate datasources — https://docs.gomplate.ca/datasources/
24. j2cli README — https://github.com/kolypto/j2cli/blob/master/README.md
25. GNU gettext, envsubst invocation — https://www.gnu.org/software/gettext/manual/html_node/envsubst-Invocation.html
26. pandoc filters — https://pandoc.org/filters.html
27. Quarto, Includes — https://quarto.org/docs/authoring/includes.html
28. Quarto, Project code execution (freeze) — https://quarto.org/docs/projects/code-execution.html
29. Quarto, Get started with a text editor (preview) — https://quarto.org/docs/get-started/hello/text-editor.html
30. R Markdown Cookbook, chunk options (cache) source — https://github.com/yihui/rmarkdown-cookbook/blob/master/11-chunk-options.Rmd
31. MyST directives reference — https://mystmd.org/guide/directives
32. MyST, reuse Jupyter outputs — https://mystmd.org/guide/reuse-jupyter-outputs
33. Org manual, Results of Evaluation — https://orgmode.org/manual/Results-of-Evaluation.html
34. Org manual, Evaluating Code Blocks — https://orgmode.org/manual/Evaluating-Code-Blocks.html
35. Org manual, Code Evaluation Security — https://orgmode.org/manual/Code-Evaluation-Security.html
36. Obsidian Dataview docs — https://blacksmithgu.github.io/obsidian-dataview/
37. Templater docs — https://silentvoid13.github.io/Templater/
38. Templater, System user functions — https://silentvoid13.github.io/Templater/user-functions/system-user-functions.html
39. Logseq docs, Queries — https://github.com/logseq/docs/blob/master/pages/Queries.md
40. Logseq docs, Advanced Queries — https://github.com/logseq/docs/blob/master/pages/Advanced%20Queries.md
41. Observable Framework, Data loaders — https://observablehq.com/framework/data-loaders (source: https://github.com/observablehq/framework/blob/main/docs/data-loaders.md)
42. Observable Framework, Getting started — https://observablehq.com/framework/getting-started
43. Observable Framework, Configuration (interpreters) — https://observablehq.com/framework/config
44. marimo, Reactivity — https://docs.marimo.io/guides/reactivity/
45. marimo, Watching for changes — https://docs.marimo.io/guides/editor_features/watching/
46. marimo, mo.watch API — https://docs.marimo.io/api/watch/
47. nbconvert usage — https://nbconvert.readthedocs.io/en/latest/usage.html
48. watchexec README — https://github.com/watchexec/watchexec/blob/main/README.md
49. watchexec man page — https://github.com/watchexec/watchexec/blob/main/doc/watchexec.1.md
50. entr(1) — https://eradman.com/entrproject/entr.1.html
51. fswatch, Invoking fswatch — https://emcrisostomo.github.io/fswatch/doc/1.17.1/fswatch.html/Invoking-fswatch.html
52. chokidar README — https://github.com/paulmillr/chokidar/blob/main/README.md
53. notify crate docs — https://docs.rs/notify/latest/notify/ (license: https://github.com/notify-rs/notify/blob/main/notify/Cargo.toml)
54. Watchman, trigger command — https://facebook.github.io/watchman/docs/cmd/trigger.html
55. Watchman, configuration (settle) — https://facebook.github.io/watchman/docs/config.html
56. Watchman, clockspec — https://facebook.github.io/watchman/docs/clockspec.html
57. inotify(7) — https://man7.org/linux/man-pages/man7/inotify.7.html
58. rename(2) — https://man7.org/linux/man-pages/man2/rename.2.html
59. GNU make manual, Rules — https://www.gnu.org/software/make/manual/html_node/Rules.html
60. just manual — https://just.systems/man/en/
61. redo, FAQ: semantics ($3, redo-stamp) — https://redo.readthedocs.io/en/latest/FAQSemantics/
62. redo documentation index — https://redo.readthedocs.io/en/latest/
63. tup, A first Tupfile — https://gittup.org/tup/ex_a_first_tupfile.html
64. Ninja manual (restat) — https://ninja-build.org/manual.html
65. GNU Stow manual — https://www.gnu.org/software/stow/manual/stow.html
66. home-manager, modules/files.nix — https://github.com/nix-community/home-manager/blob/master/modules/files.nix
67. Nix manual, nix-build — https://nix.dev/manual/nix/stable/command-ref/nix-build.html
68. Bazel, Output directory layout — https://bazel.build/remote/output-directories
69. git-config, core.symlinks — https://git-scm.com/docs/git-config#Documentation/git-config.txt-coresymlinks (verified against local `man git-config`)
70. git-fast-import, filemodify mode 120000 — https://git-scm.com/docs/git-fast-import
71. Claude Code docs, How Claude remembers your project — https://code.claude.com/docs/en/memory
72. mermaid-cli README — https://github.com/mermaid-js/mermaid-cli/blob/master/README.md
73. csvkit, csvlook — https://csvkit.readthedocs.io/en/latest/scripts/csvlook.html
74. qsv README — https://github.com/dathere/qsv/blob/master/README.md
75. SQLite CLI — https://sqlite.org/cli.html
76. DuckDB CLI output formats — https://duckdb.org/docs/current/clients/cli/output_formats.html
77. tree(1) man page source — https://github.com/Old-Man-Programmer/tree/blob/master/doc/tree.1
78. D2 README — https://github.com/terrastruct/d2/blob/master/README.md
79. Graphviz command-line invocation — https://graphviz.org/doc/info/command.html
80. Vega-Lite, Compile CLI — https://vega.github.io/vega-lite/usage/compile.html
81. jq manual — https://jqlang.org/manual/
82. Style Dictionary architecture — https://styledictionary.com/info/architecture/
83. Style Dictionary file headers — https://styledictionary.com/reference/hooks/file-headers/
84. rehype README — https://github.com/rehypejs/rehype/blob/main/readme.md
85. Prettier, Ignoring code — https://prettier.io/docs/ignore
86. pre-commit — https://pre-commit.com/
87. GitHub REST API `GET /repos/{owner}/{repo}` (license, pushed_at, archived), queried 2026-09-02 for each repository above — https://docs.github.com/rest/repos/repos#get-a-repository
88. GitHub REST API, Get repository content (symlink behaviour) — https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28
89. GitHub Changelog, GitHub Pages: deprecating symlinks in non-Actions builds — https://github.blog/changelog/2023-02-21-github-pages-deprecating-symlinks-in-non-actions-builds/
90. Local experiment, 2026-09-02, macOS/git: repo with `.computed/` gitignored, `CLAUDE.md -> .computed/CLAUDE.md`; `git ls-tree` shows `120000 blob` whose content is `.computed/CLAUDE.md`; fresh `git clone` yields a dangling link; Claude Code's Read tool on the symlink returned the target content; after `mv tmp .computed/CLAUDE.md` the symlink read the new content while a pre-existing hardlink still read the old content.
