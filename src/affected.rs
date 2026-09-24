//! `affected`: the regions whose snapshots read a path. Runs the snapshot
//! step only, never `load`, so it is as safe as `check` on an unvetted clone.
//!
//! A region reaches what its snapshot read, and, so that a path that is new
//! or already gone still answers, what its opener says it would read: the
//! directory a tree lists, down to its depth and past dotfiles only with
//! `all`; the paths an `inputs=` or `index` glob matches or lies under, the
//! literal path of a projected input; a `file` or `value` region's `src=`;
//! a `toc`'s own template; and a `use` region's `computed.toml`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::loader::{Loader, Production};
use crate::marker::Region;
use crate::render::Loaders as _;
use crate::report;
use crate::survey::{self, FileError, Template};

/// What one region reads, as far as its opener and one snapshot tell.
pub struct Reach {
    /// The canonical files the snapshot read.
    pub files: BTreeSet<PathBuf>,
    /// A tree's listing.
    pub listing: Option<Listing>,
    /// `inputs=` globs as written, against the canonical region root.
    pub globs: Vec<String>,
    pub root: PathBuf,
    /// A `file` or `value` region's `src=`, anchored.
    pub src: Option<PathBuf>,
    /// A `toc`'s own template, whose headings it lists, anchored.
    pub own: Option<PathBuf>,
    /// The `computed.toml` a `use` region's recipe comes from.
    pub recipe: Option<PathBuf>,
}

/// The directory a tree lists and what bounds the listing.
pub struct Listing {
    pub dir: PathBuf,
    pub depth: Option<usize>,
    pub all: bool,
}

/// Takes the region's snapshot through `loaders`, which must be the
/// template's, and reads its opener. A snapshot that fails leaves the
/// opener's word alone.
pub fn reach(template: &Template, region: &Region, loaders: &mut Production) -> Reach {
    loaders.take_read();
    let _ = loaders.snapshot(region);
    let root = survey::anchor(&template.ctx.region_root);
    let mut reach = Reach {
        files: loaders.take_read(),
        listing: None,
        globs: Vec::new(),
        src: None,
        own: None,
        recipe: None,
        root,
    };
    if region.opener.recipe().is_some() || region.opener.loader == "use" {
        reach.recipe = template.recipes.read.clone();
    }
    let at = |p: &Path| survey::anchor(&template.ctx.region_root.join(p));
    match Loader::from_opener(&region.opener) {
        Ok(Loader::Tree(args)) => {
            reach.listing = Some(Listing {
                dir: at(&args.src),
                depth: args.depth,
                all: args.all,
            });
        }
        Ok(Loader::Exec(args)) => reach.globs = args.inputs.unwrap_or_default(),
        Ok(Loader::Index(args)) => reach.globs = args.src,
        Ok(Loader::File(args)) => reach.src = Some(at(&args.src)),
        Ok(Loader::Value(args)) => reach.src = Some(at(&args.src)),
        Ok(Loader::Toc(_)) => reach.own = Some(survey::anchor(&template.file)),
        Err(_) => {}
    }
    reach
}

impl Reach {
    /// Whether a change at `path`, anchored, can change the snapshot: the
    /// path, or anything under it when it is a directory, is read or would
    /// be listed or matched.
    pub fn affects(&self, path: &Path) -> bool {
        let within = |read: &Path| read.starts_with(path);
        if self.files.iter().any(|f| within(f))
            || [&self.src, &self.own, &self.recipe]
                .iter()
                .any(|p| p.as_deref().is_some_and(within))
        {
            return true;
        }
        if let Some(l) = &self.listing {
            if within(&l.dir) {
                return true;
            }
            if let Ok(rel) = path.strip_prefix(&l.dir) {
                let deep = l.depth.is_some_and(|d| rel.components().count() > d);
                let hidden = !l.all
                    && rel
                        .components()
                        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
                return !deep && !hidden;
            }
        }
        let rel = survey::relative(&self.root, path);
        self.globs.iter().any(|glob| glob_reaches(glob, &rel))
    }
}

/// Whether an `inputs=` glob reaches `rel`, a path from the region root:
/// the glob matches it or a directory above it, or `rel` is a directory the
/// glob's literal start lies in. A projected entry, `path#kind=value`,
/// reaches as its literal path does.
fn glob_reaches(glob: &str, rel: &Path) -> bool {
    let glob = input_path(glob);
    let prefix: PathBuf = glob
        .split('/')
        .take_while(|c| !c.contains(['*', '?', '[', '{', '\\']))
        .collect();
    if survey::normalise(&prefix).starts_with(rel) {
        return true;
    }
    let Ok(matcher) = globset::GlobBuilder::new(glob)
        .literal_separator(true)
        .build()
        .map(|g| g.compile_matcher())
    else {
        return false;
    };
    rel.ancestors()
        .take_while(|a| !a.as_os_str().is_empty())
        .any(|a| matcher.is_match(a))
}

/// An `inputs=` entry without its projection, trimmed as globs are.
pub fn input_path(entry: &str) -> &str {
    let entry = entry.trim();
    crate::project::split_input(entry)
        .map_or(entry, |(path, _)| path)
        .trim_end_matches('/')
}

/// Lists every region whose snapshot reads under any of `paths`, templates
/// found from the current directory. A query: exit 0 whatever it finds, 2
/// when a template could not be read.
pub fn main(paths: &[PathBuf], json: bool) -> Result<u8, String> {
    let targets: Vec<PathBuf> = paths.iter().map(|p| survey::anchor(p)).collect();
    let (templates, errors) = survey::templates(&[])?;
    let mut hits: Vec<(&Template, &Region)> = Vec::new();
    for t in &templates {
        let mut loaders = t.loaders();
        for region in t.regions() {
            let reach = reach(t, region, &mut loaders);
            if targets.iter().any(|p| reach.affects(p)) {
                hits.push((t, region));
            }
        }
    }
    let exit = if errors.is_empty() { 0 } else { 2 };
    if json {
        let regions: Vec<String> = hits
            .iter()
            .map(|(t, r)| {
                format!(
                    "{{\"path\":{},\"line\":{},\"name\":{},\"loader\":{}}}",
                    report::string(&t.path.display().to_string()),
                    r.line,
                    r.opener
                        .name
                        .as_deref()
                        .map_or("null".to_string(), report::string),
                    report::string(&r.opener.loader)
                )
            })
            .collect();
        println!(
            "{{\"exit\":{exit},\"errors\":{},\"regions\":[{}]}}",
            errors_json(&errors),
            regions.join(",")
        );
        return Ok(exit);
    }
    print_errors(&errors);
    let width = hits
        .iter()
        .map(|(_, r)| r.opener.name.as_deref().map_or(0, |n| n.chars().count()))
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (t, r) in &hits {
        let name = r.opener.name.as_deref().unwrap_or("");
        writeln!(
            out,
            "{}:{} {name:width$} {}",
            t.path.display(),
            r.line,
            r.opener.loader
        )
        .unwrap();
    }
    print!("{out}");
    Ok(exit)
}

/// The file errors as report lines on stderr.
pub fn print_errors(errors: &[FileError]) {
    for e in errors {
        eprint!("{}", report::error(&e.path, e.line, &e.message));
    }
}

/// The file errors as a JSON array.
pub fn errors_json(errors: &[FileError]) -> String {
    let items: Vec<String> = errors
        .iter()
        .map(|e| {
            format!(
                "{{\"path\":{},\"line\":{},\"message\":{}}}",
                report::string(&e.path.display().to_string()),
                e.line.map_or("null".to_string(), |l| l.to_string()),
                report::string(&e.message)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glob_reaches_what_it_matches_what_lies_under_a_match_and_its_base() {
        let reaches = |g: &str, p: &str| glob_reaches(g, Path::new(p));
        assert!(reaches("docs/*.md", "docs/new.md"));
        assert!(!reaches("docs/*.md", "docs/adr/x.md"));
        assert!(reaches("docs/*.md", "docs"));
        assert!(reaches("docs/*.md", ""));
        assert!(!reaches("docs/*.md", "src"));
        assert!(reaches("src", "src/deep/a.rs"));
        assert!(reaches("src/", "src/a.rs"));
        assert!(reaches("**/*.rs", "a/b/c.rs"));
        assert!(reaches("../CLAUDE.md", "../CLAUDE.md"));
        assert!(reaches("Cargo.toml#key=package.version", "Cargo.toml"));
        assert!(reaches("docs/a.md#section=A b", "docs"));
        assert!(!reaches("Cargo.toml#key=package.version", "Cargo.lock"));
    }
}
