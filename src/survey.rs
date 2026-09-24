//! What the commands that inspect regions without rendering them share: the
//! templates an invocation covers, read and parsed as `run` reads them, the
//! paths they print, and the `git` binary.

use std::path::{Component, Path, PathBuf};

use crate::config::{self, Expansion};
use crate::loader::{Ctx, Production};
use crate::marker::{self, File, Region, Segment};

/// A template read and parsed, with the context its paths resolve in.
pub struct Template {
    /// The path as discovery or the invocation named it.
    pub path: PathBuf,
    /// The file itself: a symlinked template is its target.
    pub file: PathBuf,
    pub text: String,
    /// The parse, `use` regions expanded to their recipes' openers.
    pub parsed: File,
    pub ctx: Ctx,
    /// What expanding the recipes came to.
    pub recipes: Expansion,
}

impl Template {
    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        regions(&self.parsed)
    }

    /// Fresh loaders for this template, as `run` would build them.
    pub fn loaders(&self) -> Production {
        Production::new(self.ctx.clone()).with_recipes(&self.recipes)
    }
}

/// A file-level error: at a line of the file, or the file itself. Tier 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

/// The regions of a parsed file, in file order.
pub fn regions(file: &File) -> impl Iterator<Item = &Region> {
    file.segments.iter().filter_map(|s| match s {
        Segment::Region(r) => Some(r),
        Segment::Prose(_) => None,
    })
}

/// The file a template path names. A symlinked template is its target: its
/// paths resolve against the target's directory, the target's repository
/// decides its trust, and a write lands in it.
pub fn target(path: &Path) -> std::io::Result<PathBuf> {
    if crate::cli::is_link(path) {
        path.canonicalize()
    } else {
        Ok(path.to_path_buf())
    }
}

/// Reads one file as `run` does: `None` when it holds no region, whatever
/// its encoding; an error when it has markers and is not UTF-8 or does not
/// parse. `use` regions are expanded, as `run` expands them.
pub fn read(path: &Path) -> Result<Option<Template>, FileError> {
    let fail = |line, message: String| FileError {
        path: path.to_path_buf(),
        line,
        message,
    };
    let file = target(path).map_err(|e| fail(None, format!("unreadable: {e}")))?;
    let bytes = std::fs::read(&file).map_err(|e| fail(None, format!("unreadable: {e}")))?;
    let syntax = marker::Syntax::for_path(&file);
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) if marker::has_marker(&String::from_utf8_lossy(e.as_bytes()), syntax) => {
            return Err(fail(None, "not UTF-8".to_string()));
        }
        Err(_) => return Ok(None),
    };
    if !syntax.may_hold(&text) {
        return Ok(None);
    }
    let mut parsed = marker::parse_as(&text, syntax).map_err(|e| fail(Some(e.line), e.message))?;
    if regions(&parsed).next().is_none() {
        return Ok(None);
    }
    let ctx = Ctx::for_template(&file);
    let recipes = config::expand(&mut parsed, &ctx.region_root, ctx.repo_root.as_deref());
    Ok(Some(Template {
        path: path.to_path_buf(),
        ctx,
        file,
        text,
        parsed,
        recipes,
    }))
}

/// Every template under `paths` (the current directory when empty), found
/// as `run` finds them, and the files that could not be read.
pub fn templates(paths: &[PathBuf]) -> Result<(Vec<Template>, Vec<FileError>), String> {
    let files = crate::cli::discover(paths).map_err(|e| format!("{e:#}"))?;
    let mut templates = Vec::new();
    let mut errors = Vec::new();
    for path in files {
        match read(&path) {
            Ok(Some(t)) => templates.push(t),
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }
    Ok((templates, errors))
}

/// The region's name in reports: its `name=`, else `loader@line`.
pub fn region_name(region: &Region) -> String {
    region
        .opener
        .name
        .clone()
        .unwrap_or_else(|| format!("{}@{}", region.opener.loader, region.line))
}

/// `path` made absolute against the current directory, `.` and `..` taken
/// out, and the longest part of it that exists resolved to its canonical
/// form. A path that no longer exists, or does not yet, still compares
/// with the canonical paths snapshots read.
pub fn anchor(path: &Path) -> PathBuf {
    let absolute = std::env::current_dir()
        .map(|d| d.join(path))
        .unwrap_or_else(|_| path.to_path_buf());
    let lexical = normalise(&absolute);
    for ancestor in lexical.ancestors() {
        if let Ok(canon) = ancestor.canonicalize() {
            let rest = lexical.strip_prefix(ancestor).unwrap_or(Path::new(""));
            return if rest.as_os_str().is_empty() {
                canon
            } else {
                canon.join(rest)
            };
        }
    }
    lexical
}

/// `.` components dropped and each `..` taking out the component before it.
pub fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// `to` relative to the directory `from`, both absolute, with `..` where
/// `to` lies outside `from`.
pub fn relative(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for c in &to[common..] {
        out.push(c);
    }
    out
}

/// A canonical path as the invocation would spell it: relative to the
/// current directory.
pub fn display(path: &Path) -> String {
    let here = std::env::current_dir()
        .and_then(|d| d.canonicalize())
        .unwrap_or_default();
    let rel = relative(&here, path);
    if rel.as_os_str().is_empty() {
        ".".to_string()
    } else {
        rel.display().to_string()
    }
}

/// Runs `git` in `dir`, pinned as the `git` loader runs it, and returns its
/// stdout; its stderr is the error.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = crate::git::command(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git {}: {}", args.join(" "), stderr.trim()));
    }
    String::from_utf8(out.stdout)
        .map_err(|_| format!("git {}: output is not UTF-8", args.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_climbs_out_with_dotdot() {
        let r = |a: &str, b: &str| relative(Path::new(a), Path::new(b));
        assert_eq!(r("/a/b", "/a/b/c"), Path::new("c"));
        assert_eq!(r("/a/b", "/a/c/d"), Path::new("../c/d"));
        assert_eq!(r("/a/b", "/a/b"), Path::new(""));
    }

    #[test]
    fn normalise_takes_out_dot_and_dotdot() {
        assert_eq!(normalise(Path::new("/a/./b/../c")), Path::new("/a/c"));
        assert_eq!(normalise(Path::new("../x")), Path::new("../x"));
    }
}
