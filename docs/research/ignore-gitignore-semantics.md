# `ignore` crate vs `tree --gitignore`: walk and ignore semantics

Researched 2026-09-02 against `ignore` 0.4.33 (docs.rs + ripgrep master source) and
`tree` v2.3.2 (2026-03-16, OldManProgrammer/unix-tree master source + man page).

## Summary

Recommended `WalkBuilder` for a deterministic, git-faithful tree loader:

```rust
let mut b = ignore::WalkBuilder::new(root);
b.hidden(true)            // default; drop dotfiles like `tree` without -a. Note: git itself does NOT hide dotfiles; use hidden(false) if the listing should mirror `git ls-files`.
 .ignore(false)           // do not read `.ignore` (not a git concept)           [1]
 .parents(true)           // default; honour .gitignore files above the root      [1]
 .git_ignore(true)        // default; nested .gitignore files, git precedence      [1]
 .git_exclude(false)      // .git/info/exclude is per-clone, never committed: it would make the hash differ between machines. Set true only if "match this checkout's git status" is the goal.
 .git_global(false)       // core.excludesFile is per-user: same reason; tree never reads it either [8]
 .require_git(true)       // default; outside a repo no gitignore rules apply (tree differs, see Q4)
 .follow_links(false)     // default; matches tree without -l
 .max_depth(depth)        // Some(n): root is depth 0, same counting as tree -L n
 .sort_by_file_name(|a, b| a.cmp(b)); // byte order on OsStr: stable, locale-free; required for determinism
for entry in b.build() { /* sequential walker only; WalkParallel is unordered */ }
```
Do not call `add_custom_ignore_filename(".rgignore")`; that is ripgrep's own addition, not the crate's default [6].

Unavoidable differences from `tree --gitignore`: (a) tree sorts with `strcoll` under the user's
`LC_COLLATE`, so its order is locale dependent while the byte-order sorter above is not [8]; (b) tree's
gitignore matcher is a home-grown glob with no precedence between nested files, `|` alternation, and
looser `**` handling, so exotic patterns can filter differently [11][8]; (c) tree reads `.git/info/exclude`
and `$GIT_DIR/info/exclude` but never the global excludes file [8][11]; (d) tree applies `.gitignore`
files even outside a git repo (it walks up to `/`), `ignore` with `require_git(true)` does not [11][3];
(e) both print a directory whose children were all filtered, so this one is not a difference (Q8).

## Comparison table

| # | Topic | `ignore` crate (0.4.33) | `tree` (2.3.2) | Sources |
|---|---|---|---|---|
| 1 | Hidden files | Excluded by default. `hidden(yes)`: "Enables ignoring hidden files. This is enabled by default." Hidden = base name starts with `.`; on Windows also the HIDDEN attribute. `hidden(false)` includes them. | Excluded by default. `-a`: "All files are printed. By default tree does not print hidden files (those beginning with a dot `.`)." Code: `if (!flag.a && ent->name[0] == '.') show = false;` | [1][5][7][10] |
| 2 | Nested `.gitignore` | Honoured (`git_ignore`, default on). "Within each precedence level, more nested ignore files have a higher precedence than less nested ignore files." `parents(true)` (default) also reads ignore files in directories above the root. | Honoured: `push_files()` is called per directory and pushes `new_ignorefile(dir, ...)` onto a filter stack; at the top it calls `gitignore_search()` which walks up to `/` or the first dir containing `.git`. But `filtercheck()` returns true if ANY `remove` pattern on the whole stack matches and NO `!` pattern anywhere on the stack matches, so a deeper file cannot override a shallower one in git's last-match-wins sense. | [1][2][8][11] |
| 3 | `.git/info/exclude` and `core.excludesFile` | Both honoured by default: `git_exclude(yes)` "Enables reading `.git/info/exclude` files"; `git_global(yes)` "Enables reading a global gitignore file, whose path is specified in git's `core.excludesFile` config option." Global path resolution: `$HOME/.gitconfig` (or `GIT_CONFIG_GLOBAL`), else `$XDG_CONFIG_HOME/git/ignore`, else `$HOME/.config/git/ignore`. Worktrees: a `.git` file with `gitdir:` is followed and `commondir` resolved. | `.git/info/exclude` honoured when a `.git` directory is found while walking up; `$GIT_DIR/info/exclude` if that env var is set. Global excludes: not read. Source comment: `/* Not going to implement git configs so no core.excludesFile support. */` | [1][3][4][7][8][11] |
| 4 | Outside a git repo | `require_git(yes)`: "Whether a git repository is required to apply git-related ignore rules (global rules, .gitignore and local exclude rules). When disabled, git-related ignore rules are applied even when searching outside a git repository." Default true: `.gitignore`, exclude and global are all skipped when no `.git` (dir or file) or `.jj` is found in the root or its parents. | No repo check. `gitignore_search()` "Search up the directory tree for .gitignore files, stopping at a directory that contains a .git directory, or at /, whichever occurs first." Any `.gitignore` found on the way is applied. | [1][3][11] |
| 5 | `.ignore` / `.rgignore` | `.ignore` read by default: `ignore(yes)` "Enables reading `.ignore` files ... This is enabled by default." It outranks every `.gitignore`: "any `.ignore` file overrides all `.gitignore` files." `.rgignore` is NOT part of the crate; ripgrep adds it with `builder.add_custom_ignore_filename(".rgignore")`. To match git: `ignore(false)` and never add custom names. | No equivalent. `--gitfile=file` loads one extra file "explicitly as a gitignore file"; `-I pattern` is the ad-hoc exclude. | [1][2][6][7] |
| 6 | Symlinked directories | Not followed by default (`follow_links: false` in `WalkBuilder::new`). `follow_links(yes)`: "Whether to follow symbolic links or not." Loop detection comes from walkdir. A root path that is a file is always followed (`follow_links || p.is_file()`). | Not followed by default. `-l`: "Follows symbolic links if they point to directories, as if they were directories. Symbolic links that will result in recursion are avoided when detected." Loop detection via `findino()/saveino()` on (inode, dev); the entry is printed with "recursive, not followed". Descent condition: `(!(*dir)->lnk \|\| ((*dir)->lnk && flag.l))`. | [1][2][7][9] |
| 7 | Ordering | Default: no sorter (`sorter: None`); entries come in whatever order walkdir/`read_dir` returns, i.e. filesystem dependent. `sort_by_file_name(cmp)`: "If a compare function is set, the resulting iterator will return all paths in sorted order. The compare function will be called to compare names from entries from the same directory using only the name of the entry." `sort_by_file_path` is the same with `&Path`. Both are passed to `WalkDir::sort_by`; "Note that this is not used in the parallel iterator." The comparator is user supplied, so `OsStr::cmp` gives byte order independent of locale. | Default `basesort = alnumsort`, which is `strcoll((*a)->name,(*b)->name)` after `setlocale(LC_COLLATE, "")`: locale sensitive (CHANGES 1.6.0: "Use strcoll() instead of strcmp() to sort files based on locale if set."). `-v` = `strverscmp`; `-t`/`-c` mtime/ctime with `strcoll` tiebreak; `-r` reverses; `-U` sets `basesort = NULL` ("Do not sort. Lists files in directory order. Disables --dirsfirst."). `--dirsfirst`/`--filesfirst` are "meta-sorts": `topsort` partitions by `isdir` then calls `basesort`. | [1][2][7][8][12] |
| 8 | Directory emptied by filtering | Still yielded. The directory entry is returned before its children are examined (`WalkEvent::Dir` path in `Walk::next`); children are filtered one by one afterwards, and nothing retroactively removes the parent. | Still printed. Empty dirs are removed only with `--prune` ("Makes tree prune empty directories from the output, useful when used in conjunction with -P or -I."), implemented in `getfulltree` as `if (flag.prune && (*dir)->child == NULL ...)`. Without it the directory line is printed with no children. | [2][7][8] |
| 9 | Root entry and pre-order | Yes on both counts. walkdir yields the root at depth 0; `skip_entry` starts with `if ent.depth() == 0 { return Ok(false); }` so the root is never filtered (even if hidden or ignored). Directories are yielded on `WalkEvent::Dir` before descent (pre-order, depth first); `Exit` events pop the ignore stack. | Root printed first (`lc.printfile(dirname[i], ...)`), then `listdir(dirname[i], dir, 1, ...)`. Directory line precedes its children (pre-order). | [2][9] |
| 10 | Max depth | `max_depth(Some(n))`: "The maximum depth to recurse. The default, `None`, imposes no depth restriction." Root is depth 0, its children depth 1; at `ent.depth() == max_depth` the directory is yielded but its ignore files are not loaded and nothing below is visited. So `Some(1)` = root + immediate children. | `-L level`: "Max display depth of the directory tree." Parsed as `Level = strtoul(sLevel)-1`; children of the root are listed at `lev = 1` and descent stops when `lev > Level`. So `-L 1` = root + immediate children, identical counting to `max_depth(Some(1))`. | [1][2][7][8][9] |
| 11 | Parallel walker | `build_parallel()` returns `WalkParallel`, which "uses multiple threads for traversing a directory" and runs a closure per entry; no ordering guarantee is documented and the sorter is ignored. Workers use a LIFO stack so each is depth-first, but interleaving across threads is arbitrary. Use `build()` or collect and sort. | n/a (single threaded). | [1][2] |
| 12 | Documented deviations from git | The module states it "implements the specification as described in the `gitignore` man page from scratch" and "does *not* shell out to the `git` command line tool." No `deviat`/`differ`/`not supported` comments exist. Notable implementation notes: `core.excludesfile` is found with a regex, "the lazy approach, and isn't technically correct, but probably works in more circumstances" (no INI parser); trailing whitespace is trimmed unless the line ends in `\ ` (git semantics); `/` in a glob is a literal separator; a pattern without `/` gets a `**/` prefix; `x/**` becomes `x/**/*` so it matches contents only; `\!`, `\#` and escaped trailing `/` are handled. Match precedence across file types (`.ignore` > `.gitignore` > exclude > global) is itself a deviation from git only when `.ignore` files exist. | tree's matcher is its own `patmatch()`: `|` alternation (not in gitignore), `**` "Support" that is "mostly the same as *", character classes with `[^...]`, `?`, and a trailing `/` meaning directories only. Patterns are `relative` (no `/` or only a trailing `/`) and matched against the base name in every directory, else joined to the ignore file's directory and matched against the full path. Negation `!` is global across the stack (see Q2). CHANGES 2.3.0 reworked the search for the git root; 2.3.2 "Fix regression (maybe) in --gitignore for paths that are relative to the .gitignore file". | [4][7][8][11][12] |

## Surprising points

- `.ignore` is on by default in the crate and beats every `.gitignore` regardless of nesting. A stray
  `.ignore` in the repo would silently change the listing; `ignore(false)` is mandatory for git parity [1][2].
- `require_git(true)` (the default) also disables the *global* excludes file outside a repo, and the crate
  only recognises a repo by `.git` (dir or `gitdir:` file) or `.jj` in the root or one of its parents [3].
- tree's `--gitignore` is far from git-exact: no precedence between nested files, `!` negations apply from any
  file on the stack, no global excludes, works outside repos, `|` is alternation. Expect divergences on
  repos that rely on negation patterns or `**` in the middle of a path [11][8].
- tree's default order is `strcoll`, so `LC_ALL=C tree` and `LANG=en_US.UTF-8 tree` can list the same directory
  differently (case folding, punctuation). `OsStr::cmp` byte order equals the C locale [8].
- Both tools count depth the same way (root = 0, `-L 1` shows one level of children), so `max_depth(Some(n))`
  and `tree -L n` agree [2][9].
- Both print a directory whose children were all ignored; a gitignore'd subtree only vanishes if the directory
  itself matches a pattern (e.g. `target/`). Use `tree --prune` if you ever want the other behaviour [8].
- The crate never filters the root path itself, even if it is a dotdir or matched by an ignore rule [2].
- Hidden filtering in the crate is a name check, not a git concept: git tracks dotfiles normally. If the loader
  is meant to mirror "what git would track", `hidden(false)` is the git-faithful choice; `hidden(true)` is the
  `tree`-faithful choice. Either way, `.git/` itself is not special-cased by the crate: with `hidden(false)`
  it will be walked unless you add an override or `filter_entry` for it [3][5].
- `git_global` depends on `$HOME`, `$XDG_CONFIG_HOME`, `GIT_CONFIG_GLOBAL` and the user's gitconfig; `git_exclude`
  on a file that is not committed. Both undermine a cross-machine freshness hash; hence the recommendation to
  disable them even though git honours them [4][1].

## Sources

1. docs.rs, `ignore::WalkBuilder` (0.4.33): https://docs.rs/ignore/latest/ignore/struct.WalkBuilder.html
2. ripgrep source, `crates/ignore/src/walk.rs`: https://github.com/BurntSushi/ripgrep/blob/master/crates/ignore/src/walk.rs
3. ripgrep source, `crates/ignore/src/dir.rs`: https://github.com/BurntSushi/ripgrep/blob/master/crates/ignore/src/dir.rs
4. ripgrep source, `crates/ignore/src/gitignore.rs`: https://github.com/BurntSushi/ripgrep/blob/master/crates/ignore/src/gitignore.rs
5. ripgrep source, `crates/ignore/src/pathutil.rs`: https://github.com/BurntSushi/ripgrep/blob/master/crates/ignore/src/pathutil.rs
6. ripgrep source, `crates/core/flags/hiargs.rs` (`.rgignore` registration): https://github.com/BurntSushi/ripgrep/blob/master/crates/core/flags/hiargs.rs
7. tree man page, `doc/tree.1` (v2.3.2): https://gitlab.com/OldManProgrammer/unix-tree/-/blob/master/doc/tree.1
8. tree source, `tree.c` (version string, sorting, `push_files`, `patmatch`, prune): https://gitlab.com/OldManProgrammer/unix-tree/-/blob/master/tree.c
9. tree source, `list.c` (traversal, `-L`, `-l`): https://gitlab.com/OldManProgrammer/unix-tree/-/blob/master/list.c
10. tree source, `file.c` (`-a`, per-entry filtering): https://gitlab.com/OldManProgrammer/unix-tree/-/blob/master/file.c
11. tree source, `filter.c` (`gitignore_search`, `filtercheck`, `new_pattern`): https://gitlab.com/OldManProgrammer/unix-tree/-/blob/master/filter.c
12. tree `CHANGES` (2.3.2, 2026-03-16): https://gitlab.com/OldManProgrammer/unix-tree/-/blob/master/CHANGES
