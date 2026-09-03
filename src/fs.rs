//! The walk, the repository root, and the atomic write.

use std::io;
use std::path::{Path, PathBuf};

/// Options for a walk, named after the `tree` loader's attributes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkOpts {
    /// `None` is unlimited; `Some(n)` counts as `tree -L n` counts.
    pub depth: Option<usize>,
    /// Include dotfiles.
    pub all: bool,
    /// Directories only.
    pub dirs: bool,
}

/// One entry of a walk, below the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Relative to the walk root.
    pub path: PathBuf,
    pub is_dir: bool,
    /// 1 for the root's children.
    pub depth: usize,
}

/// Walks `root` with the `ignore` settings that keep a listing identical
/// between machines: dotfiles hidden unless `all`, nested `.gitignore`
/// honoured only inside a git repository, no per-clone or per-user exclude
/// files, no `.ignore`, no symlinks followed, byte-order names. The root
/// itself is not yielded; `.git` never is.
pub fn walk(root: &Path, opts: WalkOpts) -> impl Iterator<Item = Entry> {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(!opts.all)
        .ignore(false)
        .git_ignore(true)
        .parents(true)
        .require_git(true)
        .git_exclude(false)
        .git_global(false)
        .follow_links(false)
        .max_depth(opts.depth)
        .sort_by_file_name(|a, b| a.cmp(b));
    if opts.all {
        b.filter_entry(|e| e.file_name() != ".git");
    }
    let root = root.to_path_buf();
    b.build().filter_map(move |entry| {
        let entry = entry.ok()?;
        if entry.depth() == 0 {
            return None;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if opts.dirs && !is_dir {
            return None;
        }
        let path = entry.path().strip_prefix(&root).ok()?.to_path_buf();
        Some(Entry {
            path,
            is_dir,
            depth: entry.depth(),
        })
    })
}

/// The canonical root of the repository containing `path`: the nearest
/// ancestor (or the path itself) holding a `.git`, or `None` outside one.
pub fn repo_root(path: &Path) -> Option<PathBuf> {
    let start = path.canonicalize().ok()?;
    start
        .ancestors()
        .find(|p| p.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Writes `text` to `path` through a temp file in the same directory and
/// `rename(2)`, copying the original's mode bits. Returns `false`, touching
/// nothing, when the file already holds `text`.
pub fn write(path: &Path, text: &str) -> io::Result<bool> {
    if std::fs::read(path)
        .map(|current| current == text.as_bytes())
        .unwrap_or(false)
    {
        return Ok(false);
    }
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".computed-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    io::Write::write_all(&mut tmp, text.as_bytes())?;
    if let Ok(meta) = std::fs::metadata(path) {
        tmp.as_file().set_permissions(meta.permissions())?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::create_dir_all(r.join("src/nested/deep")).unwrap();
        fs::create_dir_all(r.join("target/debug")).unwrap();
        fs::create_dir_all(r.join("docs/tmp")).unwrap();
        fs::create_dir_all(r.join("tmp")).unwrap();
        fs::write(
            r.join(".gitignore"),
            "# one of each pattern class the tree loader promises\n\ntarget/\n*.log\n/tmp\nscratch.md\n",
        )
        .unwrap();
        fs::write(r.join(".hidden"), "").unwrap();
        fs::write(r.join("src/main.rs"), "").unwrap();
        fs::write(r.join("src/nested/deep/x.rs"), "").unwrap();
        fs::write(r.join("src/build.log"), "").unwrap();
        fs::write(r.join("src/scratch.md"), "").unwrap();
        fs::write(r.join("target/debug/bin"), "").unwrap();
        fs::write(r.join("docs/tmp/kept.md"), "").unwrap();
        fs::write(r.join("docs/target"), "").unwrap();
        fs::write(r.join("tmp/dropped.md"), "").unwrap();
        fs::write(r.join("Zed.md"), "").unwrap();
        fs::write(r.join("a.md"), "").unwrap();
        fs::write(r.join("scratch.md"), "").unwrap();
        dir
    }

    fn listing(root: &Path, opts: WalkOpts) -> Vec<String> {
        walk(root, opts)
            .map(|e| format!("{}{}", e.path.display(), if e.is_dir { "/" } else { "" }))
            .collect()
    }

    /// Literal name, `*.ext`, `/anchored`, `dir/`, a comment and a blank
    /// line: the root `.gitignore` governs the listing without any flag.
    #[test]
    fn walk_honours_gitignore_hides_dotfiles_and_sorts_by_byte_order() {
        let dir = repo();
        let got = listing(dir.path(), WalkOpts::default());
        assert_eq!(
            got,
            [
                "Zed.md",
                "a.md",
                "docs/",
                "docs/target",
                "docs/tmp/",
                "docs/tmp/kept.md",
                "src/",
                "src/main.rs",
                "src/nested/",
                "src/nested/deep/",
                "src/nested/deep/x.rs"
            ]
        );
    }

    #[test]
    fn walk_outside_a_repository_applies_no_gitignore() {
        let dir = repo();
        fs::remove_dir_all(dir.path().join(".git")).unwrap();
        let got = listing(dir.path(), WalkOpts::default());
        assert!(got.contains(&"target/".to_string()));
        assert!(got.contains(&"tmp/dropped.md".to_string()));
        assert!(got.contains(&"src/build.log".to_string()));
    }

    #[test]
    fn walk_all_includes_dotfiles_but_never_the_git_directory_contents() {
        let dir = repo();
        let got = listing(
            dir.path(),
            WalkOpts {
                all: true,
                ..Default::default()
            },
        );
        assert!(got.contains(&".hidden".to_string()));
        assert!(got.contains(&".gitignore".to_string()));
        assert!(!got.iter().any(|p| p.starts_with(".git/")), "{got:?}");
        assert!(!got.contains(&".git/".to_string()), "{got:?}");
    }

    #[test]
    fn walk_depth_counts_like_tree_and_dirs_lists_directories_only() {
        let dir = repo();
        let got = listing(
            dir.path(),
            WalkOpts {
                depth: Some(1),
                ..Default::default()
            },
        );
        assert_eq!(got, ["Zed.md", "a.md", "docs/", "src/"]);
        let got = listing(
            dir.path(),
            WalkOpts {
                dirs: true,
                ..Default::default()
            },
        );
        assert_eq!(
            got,
            [
                "docs/",
                "docs/tmp/",
                "src/",
                "src/nested/",
                "src/nested/deep/"
            ]
        );
    }

    #[test]
    fn repo_root_walks_up_for_git_and_canonicalises() {
        let dir = repo();
        let nested = dir.path().join("src/nested");
        let expected = dir.path().canonicalize().unwrap();
        assert_eq!(repo_root(&nested), Some(expected.clone()));
        assert_eq!(repo_root(&nested.join("deep/x.rs")), Some(expected));
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(repo_root(plain.path()), None);
    }

    #[test]
    fn write_is_atomic_keeps_mode_and_skips_unchanged() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.md");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(write(&path, "new").unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(!write(&path, "new").unwrap());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp file left behind"
        );
    }
}
