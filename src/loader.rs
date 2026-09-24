//! The three loaders, `tree`, `exec` and `file`, and the production
//! `Loaders` adapter.

/// What every loader produces: the text a sink shapes, and the snapshot of
/// the inputs it read, which the input sum is taken over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub text: String,
    pub snapshot: Vec<u8>,
}

/// A loader error with its exit tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Tier 2: the tool could not answer. The file is skipped whole.
    Hard(String),
    /// Tier 1: the loader ran and failed. The previous body is kept.
    Failed { stderr: String },
}

/// The per-loader format constant folded into the input sum. Bumped by hand
/// only when that loader's output for the same inputs changes; a change to a
/// normalisation rule bumps both.
pub fn format_constant(loader: &str) -> u32 {
    match loader {
        "tree" => 1,
        "exec" => 1,
        "file" => 1,
        "value" => 1,
        "index" => 1,
        "toc" => 1,
        other => panic!("unknown loader {other:?} reached the format table"),
    }
}

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use globset::GlobMatcher;

use crate::fs::{self, Ignores, WalkOpts};
use crate::index::{self, Title};
use crate::marker::{self, Opener, Region};
use crate::project::{self, Projection};
use crate::render::Loaders;
use crate::toc;

/// Per-file context every marker path is resolved against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ctx {
    /// The template path as the invocation named it.
    pub template: PathBuf,
    /// The template's directory: what relative paths resolve against and
    /// where an exec command runs.
    pub region_root: PathBuf,
    /// The canonical repository root, `None` outside one.
    pub repo_root: Option<PathBuf>,
}

impl Ctx {
    pub fn for_template(template: &Path) -> Ctx {
        let region_root = template
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        Ctx {
            template: template.to_path_buf(),
            repo_root: fs::repo_root(&region_root),
            region_root,
        }
    }

    /// The directory marker paths must stay inside, canonical.
    fn bound(&self) -> Result<PathBuf, LoadError> {
        match &self.repo_root {
            Some(r) => Ok(r.clone()),
            None => self
                .region_root
                .canonicalize()
                .map_err(|e| hard(format!("region root: {e}"))),
        }
    }

    /// Resolves a marker path against the region root and checks it exists
    /// and does not escape the bound.
    fn resolve(&self, what: &str, rel: &Path) -> Result<PathBuf, LoadError> {
        let joined = self.region_root.join(rel);
        let canon = joined
            .canonicalize()
            .map_err(|e| hard(format!("{what}: {}: {e}", rel.display())))?;
        let bound = self.bound()?;
        if !canon.starts_with(&bound) {
            return Err(hard(format!(
                "{what}: {} escapes {}",
                rel.display(),
                bound.display()
            )));
        }
        Ok(canon)
    }
}

fn hard(message: impl Into<String>) -> LoadError {
    LoadError::Hard(message.into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeArgs {
    pub src: PathBuf,
    pub depth: Option<usize>,
    pub all: bool,
    pub dirs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileArgs {
    pub src: PathBuf,
    /// `lines=`, `section=` or `anchor=`: the slice of the file taken.
    pub slice: Option<Projection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueArgs {
    pub src: PathBuf,
    /// The dotted path, one entry per component.
    pub key: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexArgs {
    /// Comma-separated globs, expanded as `inputs=` is.
    pub src: Vec<String>,
    pub title: Title,
}

/// The heading levels a toc lists, inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocArgs {
    pub min: usize,
    pub max: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecArgs {
    pub cmd: String,
    /// Comma-separated globs, or `None` when volatile.
    pub inputs: Option<Vec<String>>,
    pub timeout: Duration,
}

/// The closed set of loaders, built from an opener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loader {
    Tree(TreeArgs),
    Exec(ExecArgs),
    File(FileArgs),
    Value(ValueArgs),
    Index(IndexArgs),
    Toc(TocArgs),
}

impl Loader {
    pub fn from_opener(opener: &Opener) -> Result<Loader, LoadError> {
        match opener.loader.as_str() {
            "tree" => {
                let depth = match opener.attr("depth") {
                    None => None,
                    Some(d) => Some(
                        d.parse::<usize>()
                            .map_err(|_| hard(format!("depth={d}: expected a whole number")))?,
                    ),
                };
                Ok(Loader::Tree(TreeArgs {
                    src: PathBuf::from(opener.attr("src").unwrap_or(".")),
                    depth,
                    all: opener.flag("all"),
                    dirs: opener.flag("dirs"),
                }))
            }
            "exec" => {
                let cmd = opener
                    .attr("cmd")
                    .ok_or_else(|| hard("exec needs cmd="))?
                    .to_string();
                let inputs = opener
                    .attr("inputs")
                    .map(|i| i.split(',').map(str::to_string).collect::<Vec<_>>());
                match (&inputs, opener.flag("volatile")) {
                    (Some(_), true) => {
                        return Err(hard("exec takes inputs= or volatile, not both"));
                    }
                    (None, false) => return Err(hard("exec needs inputs= or the volatile flag")),
                    _ => {}
                }
                let timeout = match opener.attr("timeout") {
                    None => 30,
                    Some(t) => t.parse::<u64>().map_err(|_| {
                        hard(format!("timeout={t}: expected seconds as a whole number"))
                    })?,
                };
                Ok(Loader::Exec(ExecArgs {
                    cmd,
                    inputs,
                    timeout: Duration::from_secs(timeout),
                }))
            }
            "file" => Ok(Loader::File(FileArgs {
                src: PathBuf::from(opener.attr("src").ok_or_else(|| hard("file needs src="))?),
                slice: project::from_attrs(&opener.attrs, project::FILE_SLICES).map_err(hard)?,
            })),
            "value" => {
                let key = opener.attr("key").ok_or_else(|| hard("value needs key="))?;
                let Projection::Key(key) = Projection::parse("key", key).map_err(hard)? else {
                    unreachable!("a key= projection")
                };
                Ok(Loader::Value(ValueArgs {
                    src: PathBuf::from(opener.attr("src").ok_or_else(|| hard("value needs src="))?),
                    key,
                }))
            }
            "index" => {
                let src = opener.attr("src").ok_or_else(|| hard("index needs src="))?;
                let title = match opener.attr("title") {
                    None => Title::H1,
                    Some(t) => Title::parse(t)
                        .ok_or_else(|| hard(format!("title={t}: expected h1 or filename")))?,
                };
                Ok(Loader::Index(IndexArgs {
                    src: src.split(',').map(str::to_string).collect(),
                    title,
                }))
            }
            "toc" => {
                let (min, max) =
                    toc::levels(opener.attr("min"), opener.attr("max")).map_err(hard)?;
                Ok(Loader::Toc(TocArgs { min, max }))
            }
            other => Err(hard(format!("unknown loader {other:?}"))),
        }
    }

    pub fn format_constant(&self) -> u32 {
        match self {
            Loader::Tree(_) => format_constant("tree"),
            Loader::Exec(_) => format_constant("exec"),
            Loader::File(_) => format_constant("file"),
            Loader::Value(_) => format_constant("value"),
            Loader::Index(_) => format_constant("index"),
            Loader::Toc(_) => format_constant("toc"),
        }
    }
}

/// The production `Loaders` adapter: resolves paths through a `Ctx`, keeps
/// each tree walk so `snapshot` and `load` cost one walk, and records every
/// file whose content went into a snapshot.
pub struct Production {
    ctx: Ctx,
    walks: HashMap<String, Loaded>,
    read: BTreeSet<PathBuf>,
}

impl Production {
    pub fn new(ctx: Ctx) -> Production {
        Production {
            ctx,
            walks: HashMap::new(),
            read: BTreeSet::new(),
        }
    }

    /// The canonical paths of every file a snapshot read, so a caller can
    /// tell which templates a write to another file makes stale.
    pub fn read(&self) -> &BTreeSet<PathBuf> {
        &self.read
    }

    fn tree(&mut self, region: &Region, args: &TreeArgs) -> Result<Loaded, LoadError> {
        let key = region.opener.canonical();
        if let Some(l) = self.walks.get(&key) {
            return Ok(l.clone());
        }
        let src = self.ctx.resolve("src=", &args.src)?;
        if !src.is_dir() {
            return Err(hard(format!(
                "src=: {} is not a directory",
                args.src.display()
            )));
        }
        let loaded = tree(
            &src,
            WalkOpts {
                depth: args.depth,
                all: args.all,
                dirs: args.dirs,
            },
        );
        self.walks.insert(key, loaded.clone());
        Ok(loaded)
    }

    /// The `file` loader: the named file's text, closer sums taken out, and
    /// the same one-entry snapshot `inputs=` would take of it. A slice
    /// narrows both to the projected part, so an edit elsewhere in the file
    /// leaves the region fresh.
    fn file(&mut self, args: &FileArgs) -> Result<Loaded, LoadError> {
        let path = self.ctx.resolve("src=", &args.src)?;
        if !path.is_file() {
            return Err(hard(format!("src=: {} is not a file", args.src.display())));
        }
        if self.ctx.template.canonicalize().ok().as_ref() == Some(&path) {
            return Err(hard(format!(
                "src=: {} is this file; a region cannot include its own file",
                args.src.display()
            )));
        }
        let rel = lexical(&args.src);
        let content =
            std::fs::read(&path).map_err(|e| hard(format!("src=: {}: {e}", args.src.display())))?;
        let mut content = marker::strip_sums(&content).into_owned();
        self.read.insert(path);
        let mut key = rel.to_string_lossy().into_owned();
        if let Some(slice) = &args.slice {
            content = slice
                .apply(&args.src, &content)
                .map_err(|e| hard(format!("src=: {}: {e}", args.src.display())))?;
            key = format!("{key}#{}", slice.canonical());
        }
        let mut snapshot = Vec::new();
        push_entry(&mut snapshot, key.as_bytes(), &content);
        let text = String::from_utf8(content).map_err(|_| LoadError::Failed {
            stderr: format!("src=: {} is not UTF-8", args.src.display()),
        })?;
        Ok(Loaded { text, snapshot })
    }

    /// The `value` loader: one scalar of a TOML, JSON or YAML file. The
    /// snapshot is the key and the value alone, so the rest of the file can
    /// change under it.
    fn value(&mut self, args: &ValueArgs) -> Result<Loaded, LoadError> {
        let path = self.ctx.resolve("src=", &args.src)?;
        if !path.is_file() {
            return Err(hard(format!("src=: {} is not a file", args.src.display())));
        }
        let content =
            std::fs::read(&path).map_err(|e| hard(format!("src=: {}: {e}", args.src.display())))?;
        self.read.insert(path);
        let text = project::scalar(&args.src, &content, &args.key)
            .map_err(|e| hard(format!("src=: {}: {e}", args.src.display())))?;
        let entry = format!(
            "{}#key={}",
            lexical(&args.src).display(),
            args.key.join(".")
        );
        let mut snapshot = Vec::new();
        push_entry(&mut snapshot, entry.as_bytes(), text.as_bytes());
        Ok(Loaded { text, snapshot })
    }

    /// The `index` loader: a link per file the globs select, in byte order
    /// of path. The snapshot is each path with the title taken from it, the
    /// only part of a file the list depends on.
    fn index(&mut self, args: &IndexArgs) -> Result<Loaded, LoadError> {
        let mut text = String::new();
        let mut snapshot = Vec::new();
        for (rel, file) in expand(&self.ctx, "src", &args.src)? {
            let rel = String::from_utf8_lossy(&rel).into_owned();
            let content = if args.title.reads(Path::new(&rel)) {
                let read = present(std::fs::read(&file))
                    .map_err(|e| hard(format!("src: {}: {e}", file.display())))?;
                let Some(content) = read else {
                    continue;
                };
                self.read.insert(file);
                Some(content)
            } else {
                None
            };
            let title = index::title(Path::new(&rel), content.as_deref());
            text.push_str(&index::line(&title, &rel));
            text.push('\n');
            push_entry(&mut snapshot, rel.as_bytes(), title.as_bytes());
        }
        Ok(Loaded { text, snapshot })
    }

    /// The `toc` loader: the headings of the template's own prose. The one
    /// loader that reads its template; it does not record it as read, since
    /// the only write to the template in a run is the run's own, which
    /// leaves the prose, and so the toc, as it was.
    fn toc(&mut self, args: &TocArgs) -> Result<Loaded, LoadError> {
        let template = std::fs::read_to_string(&self.ctx.template)
            .map_err(|e| hard(format!("{}: {e}", self.ctx.template.display())))?;
        let (text, snapshot) = toc::toc(&template, args.min, args.max).map_err(hard)?;
        Ok(Loaded { text, snapshot })
    }

    fn region_name(&self, region: &Region) -> String {
        region
            .opener
            .name
            .clone()
            .unwrap_or_else(|| format!("{}@{}", region.opener.loader, region.line))
    }
}

impl Loaders for Production {
    fn snapshot(&mut self, region: &Region) -> Result<Option<Vec<u8>>, LoadError> {
        match Loader::from_opener(&region.opener)? {
            Loader::Tree(args) => Ok(Some(self.tree(region, &args)?.snapshot)),
            Loader::Exec(ExecArgs { inputs: None, .. }) => Ok(None),
            Loader::Exec(ExecArgs {
                inputs: Some(globs),
                ..
            }) => Ok(Some(inputs_snapshot(&self.ctx, &globs, &mut self.read)?)),
            Loader::File(args) => Ok(Some(self.file(&args)?.snapshot)),
            Loader::Value(args) => Ok(Some(self.value(&args)?.snapshot)),
            Loader::Index(args) => Ok(Some(self.index(&args)?.snapshot)),
            Loader::Toc(args) => Ok(Some(self.toc(&args)?.snapshot)),
        }
    }

    fn load(&mut self, region: &Region) -> Result<Loaded, LoadError> {
        match Loader::from_opener(&region.opener)? {
            Loader::Tree(args) => self.tree(region, &args),
            Loader::Exec(args) => {
                let snapshot = match &args.inputs {
                    None => Vec::new(),
                    Some(globs) => inputs_snapshot(&self.ctx, globs, &mut self.read)?,
                };
                let text = exec(&self.ctx, &args, &self.region_name(region))?;
                Ok(Loaded { text, snapshot })
            }
            Loader::File(args) => self.file(&args),
            Loader::Value(args) => self.value(&args),
            Loader::Index(args) => self.index(&args),
            Loader::Toc(args) => self.toc(&args),
        }
    }
}

/// One walk: the `tree`-style listing and the snapshot, the same sequence.
fn tree(src: &Path, opts: WalkOpts) -> Loaded {
    let entries: Vec<fs::Entry> = fs::walk(src, opts).collect();
    let mut snapshot = Vec::new();
    for e in &entries {
        snapshot.extend_from_slice(e.path.to_string_lossy().as_bytes());
        if e.is_dir {
            snapshot.push(b'/');
        }
        snapshot.push(b'\n');
    }
    // Children per directory, in walk order.
    let mut children: BTreeMap<PathBuf, Vec<&fs::Entry>> = BTreeMap::new();
    for e in &entries {
        let parent = e.path.parent().map(Path::to_path_buf).unwrap_or_default();
        children.entry(parent).or_default().push(e);
    }
    let mut text = String::from(".\n");
    fn draw(
        dir: &Path,
        prefix: &str,
        children: &BTreeMap<PathBuf, Vec<&fs::Entry>>,
        out: &mut String,
    ) {
        let Some(list) = children.get(dir) else {
            return;
        };
        for (i, e) in list.iter().enumerate() {
            let last = i + 1 == list.len();
            let name = e.path.file_name().unwrap_or_default().to_string_lossy();
            out.push_str(prefix);
            out.push_str(if last { "└── " } else { "├── " });
            out.push_str(&name);
            out.push('\n');
            if e.is_dir {
                let next = format!("{prefix}{}", if last { "    " } else { "│   " });
                draw(&e.path, &next, children, out);
            }
        }
    }
    draw(Path::new(""), "", &children, &mut text);
    Loaded { text, snapshot }
}

/// One `inputs=` glob, compiled whole to decide what it selects and split at
/// `/` so expansion can follow it one directory at a time: to enter only the
/// directories that could hold a match, and to tell an entry a literal
/// component named from one a wildcard reached.
struct InputGlob {
    whole: GlobMatcher,
    parts: Vec<Part>,
}

/// One `/`-separated component of an [`InputGlob`].
enum Part {
    /// `**`, or a component that does not compile alone. Either may span any
    /// run of directories, so nothing below it is pruned.
    Any,
    /// A component with no wildcard: it names what it matches.
    Literal(OsString),
    Pattern(GlobMatcher),
}

/// Where expansion stands in a glob: `0[i]` when part `i` may match the
/// next component, `0[parts.len()]` when every part has matched.
#[derive(Debug, Clone)]
struct Live(Vec<bool>);

/// The glob's state after one more component.
struct Step {
    live: Live,
    /// A literal part matched the component.
    named: bool,
}

impl InputGlob {
    fn new(glob: &str) -> Result<InputGlob, globset::Error> {
        let parts = glob
            .split('/')
            .filter(|c| !c.is_empty())
            .map(|c| match c {
                "**" => Part::Any,
                c if is_literal(c) => Part::Literal(c.into()),
                c => compile(c).map_or(Part::Any, Part::Pattern),
            })
            .collect();
        Ok(InputGlob {
            whole: compile(glob)?,
            parts,
        })
    }

    fn matches(&self, rel: &Path) -> bool {
        self.whole.is_match(rel)
    }

    /// The state before any component.
    fn start(&self) -> Live {
        let mut live = Live(vec![false; self.parts.len() + 1]);
        live.0[0] = true;
        self.skip_any(&mut live);
        live
    }

    fn step(&self, live: &Live, name: &OsStr) -> Step {
        let mut next = Live(vec![false; self.parts.len() + 1]);
        let mut named = false;
        for (i, part) in self.parts.iter().enumerate().filter(|&(i, _)| live.0[i]) {
            match part {
                Part::Any => next.0[i] = true,
                Part::Literal(l) if l == name => {
                    next.0[i + 1] = true;
                    named = true;
                }
                Part::Pattern(m) if m.is_match(name) => next.0[i + 1] = true,
                Part::Literal(_) | Part::Pattern(_) => {}
            }
        }
        self.skip_any(&mut next);
        Step { live: next, named }
    }

    /// Whether a path below the one `live` stands at could still match: a
    /// part is left over.
    fn may_contain(&self, live: &Live) -> bool {
        live.0[..self.parts.len()].contains(&true)
    }

    /// A live `**` may also match no directory at all.
    fn skip_any(&self, live: &mut Live) {
        for (i, part) in self.parts.iter().enumerate() {
            if live.0[i] && matches!(part, Part::Any) {
                live.0[i + 1] = true;
            }
        }
    }
}

/// A glob component with nothing for globset to expand. Braces are not
/// `inputs=` syntax, but globset reads them, so they count as a wildcard.
fn is_literal(component: &str) -> bool {
    !component.contains(['*', '?', '[', '{', '\\'])
}

fn compile(glob: &str) -> Result<GlobMatcher, globset::Error> {
    Ok(globset::GlobBuilder::new(glob)
        .literal_separator(true)
        .build()?
        .compile_matcher())
}

/// What expansion takes below a directory.
enum Below {
    /// Every file: a matched directory means every file under it.
    Everything,
    Glob(Live),
}

/// The files one glob expands to.
struct Expansion<'g> {
    glob: &'g InputGlob,
    /// What a symlink's target must stay inside, canonical.
    bound: &'g Path,
    repo_root: Option<&'g Path>,
    /// `(relative, path)` per file taken, the path canonical.
    files: Vec<(PathBuf, PathBuf)>,
    /// A wildcard reached an ignored path it would have taken or entered.
    ignored: bool,
}

impl<'g> Expansion<'g> {
    fn new(glob: &'g InputGlob, bound: &'g Path, repo_root: Option<&'g Path>) -> Expansion<'g> {
        Expansion {
            glob,
            bound,
            repo_root,
            files: Vec::new(),
            ignored: false,
        }
    }

    /// Takes what `below` selects from `dir`, whose path from the region
    /// root is `rel`. An entry a wildcard reached is skipped when
    /// `.gitignore` ignores it or it is `.git`; an entry a literal part named
    /// is taken regardless ([ADR 0012]). A symlink to a file is read through
    /// when its target stays inside the bound; a symlink to a directory is
    /// entered only when a literal part named it, so a wildcard never loops.
    /// A target outside the bound is an error when named and skipped when a
    /// wildcard reached it. A directory or file that vanishes while it is
    /// listed, as build output does, is skipped: it is not there to snapshot.
    ///
    /// [ADR 0012]: ../docs/adr/0012-wildcards-in-inputs-do-not-reach-ignored-paths.md
    fn walk(&mut self, dir: &Path, rel: &Path, below: &Below, ignores: &Ignores) -> io::Result<()> {
        let at = |e: io::Error| io::Error::new(e.kind(), format!("{}: {e}", dir.display()));
        let Some(listing) = present(std::fs::read_dir(dir)).map_err(at)? else {
            return Ok(());
        };
        for entry in listing {
            let Some(entry) = present(entry).map_err(at)? else {
                continue;
            };
            let Some(kind) = present(entry.file_type()).map_err(at)? else {
                continue;
            };
            let link = kind.is_symlink();
            let kind = if link {
                match present(std::fs::metadata(entry.path())).map_err(at)? {
                    Some(m) => m.file_type(),
                    None => continue,
                }
            } else {
                kind
            };
            let is_dir = kind.is_dir();
            if !is_dir && !kind.is_file() {
                continue;
            }
            let name = entry.file_name();
            let rel = rel.join(&name);
            let (next, named) = match below {
                Below::Everything => (Below::Everything, false),
                Below::Glob(live) => {
                    let step = self.glob.step(live, &name);
                    if self.glob.matches(&rel) {
                        (Below::Everything, step.named)
                    } else if is_dir && self.glob.may_contain(&step.live) {
                        (Below::Glob(step.live), step.named)
                    } else {
                        continue;
                    }
                }
            };
            let mut path = entry.path();
            if !named && (name == ".git" || ignores.ignores(&path, is_dir)) {
                self.ignored = true;
                continue;
            }
            if link {
                if is_dir && !named {
                    continue;
                }
                let Some(target) = present(path.canonicalize()).map_err(at)? else {
                    continue;
                };
                if !target.starts_with(self.bound) {
                    if named {
                        return Err(io::Error::other(format!(
                            "{} escapes {}",
                            rel.display(),
                            self.bound.display()
                        )));
                    }
                    continue;
                }
                path = target;
            }
            if is_dir {
                let inner = if link {
                    Ignores::at(self.repo_root, &path)
                } else {
                    ignores.enter(&path)
                };
                self.walk(&path, &rel, &next, &inner)?;
            } else if matches!(next, Below::Everything) {
                self.files.push((rel, path));
            }
        }
        Ok(())
    }
}

/// `None` for a path that no longer exists.
fn present<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The literal directory a glob starts in, before its first wildcard.
fn glob_prefix(glob: &str) -> PathBuf {
    let mut prefix = PathBuf::new();
    for comp in glob.split('/') {
        if !is_literal(comp) {
            break;
        }
        prefix.push(comp);
    }
    if glob.split('/').all(is_literal) {
        // A literal path names a file or directory; match from its parent.
        prefix.pop();
    }
    if prefix.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        prefix
    }
}

/// A marker path as the glob spells it, `.` components dropped, so a
/// symlinked directory keeps the name the glob matches against.
fn lexical(path: &Path) -> PathBuf {
    path.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect()
}

/// The `inputs=` snapshot: for every matched file in byte-order relative
/// path, `path NUL length NUL content NUL`, closer sums taken out of the
/// content. A matched directory means every file under it; the template
/// itself is excluded. An entry with a projection, `path#kind=value`, takes
/// the entry `path#kind=value NUL length NUL slice NUL` in the same order,
/// so an edit outside the slice leaves the snapshot as it was. Every file
/// read is added to `read`.
fn inputs_snapshot(
    ctx: &Ctx,
    globs: &[String],
    read: &mut BTreeSet<PathBuf>,
) -> Result<Vec<u8>, LoadError> {
    let mut plain = Vec::new();
    let mut matched: BTreeMap<Vec<u8>, (PathBuf, Option<Projection>)> = BTreeMap::new();
    for entry in globs {
        let entry = entry.trim();
        let (path, projection) =
            project::split_input(entry).map_err(|e| hard(format!("inputs={e}")))?;
        let Some(projection) = projection else {
            plain.push(entry.to_string());
            continue;
        };
        let rel = Path::new(path);
        if !ctx.region_root.join(rel).exists() {
            return Err(hard(format!("inputs={entry} matches nothing")));
        }
        let file = ctx.resolve("inputs=", rel)?;
        if !file.is_file() {
            return Err(hard(format!("inputs={entry}: {path} is not a file")));
        }
        if ctx.template.canonicalize().ok().as_ref() == Some(&file) {
            return Err(hard(format!(
                "inputs={entry}: {path} is this file; a region cannot read a part of its own file"
            )));
        }
        let key = format!("{}#{}", lexical(rel).display(), projection.canonical());
        matched.insert(key.into_bytes(), (file, Some(projection)));
    }
    if !plain.is_empty() {
        matched.extend(
            expand(ctx, "inputs", &plain)?
                .into_iter()
                .map(|(rel, file)| (rel, (file, None))),
        );
    }
    content_snapshot(matched, read)
}

/// The files a list of globs selects, keyed by their path from the region
/// root in byte order, each with its canonical path; the template is left
/// out. A glob that matches nothing is an error. `attr` names the attribute
/// the globs came from, for messages.
fn expand(
    ctx: &Ctx,
    attr: &str,
    globs: &[String],
) -> Result<BTreeMap<Vec<u8>, PathBuf>, LoadError> {
    let template = ctx.template.canonicalize().ok();
    let bound = ctx.bound()?;
    let mut matched: BTreeMap<Vec<u8>, PathBuf> = BTreeMap::new();
    for glob in globs {
        // `docs/` names the directory `docs` names.
        let glob = glob.trim().trim_end_matches('/');
        let input = InputGlob::new(glob).map_err(|e| hard(format!("{attr}={glob}: {e}")))?;
        let prefix = glob_prefix(glob);
        if !ctx.region_root.join(&prefix).exists() {
            return Err(hard(format!("{attr}={glob} matches nothing")));
        }
        let dir = ctx.resolve(&format!("{attr}="), &prefix)?;
        let rel = lexical(&prefix);
        let below = if input.matches(&rel) {
            Below::Everything
        } else {
            Below::Glob(rel.components().fold(input.start(), |live, c| {
                input.step(&live, c.as_os_str()).live
            }))
        };
        let ignores = Ignores::at(ctx.repo_root.as_deref(), &dir);
        let mut expansion = Expansion::new(&input, &bound, ctx.repo_root.as_deref());
        expansion
            .walk(&dir, &rel, &below, &ignores)
            .map_err(|e| hard(format!("{attr}={glob}: {e}")))?;
        if expansion.files.is_empty() {
            let why = if expansion.ignored {
                " that is not ignored"
            } else {
                ""
            };
            return Err(hard(format!("{attr}={glob} matches nothing{why}")));
        }
        for (rel, file) in expansion.files {
            if template.as_ref() != Some(&file) {
                matched.insert(rel.to_string_lossy().as_bytes().to_vec(), file);
            }
        }
    }
    Ok(matched)
}

/// The snapshot bytes over the matched files, each narrowed by its
/// projection when it has one. A file deleted since expansion listed it is
/// left out, as if the listing had missed it. The sums in another
/// template's closers are not content: they change when that file renders,
/// not when what it says does, and two templates that read each other would
/// otherwise never settle.
fn content_snapshot(
    matched: BTreeMap<Vec<u8>, (PathBuf, Option<Projection>)>,
    read: &mut BTreeSet<PathBuf>,
) -> Result<Vec<u8>, LoadError> {
    let mut out = Vec::new();
    for (key, (file, projection)) in matched {
        let Some(content) = present(std::fs::read(&file))
            .map_err(|e| hard(format!("inputs: {}: {e}", file.display())))?
        else {
            continue;
        };
        read.insert(file);
        let content = marker::strip_sums(&content);
        match projection {
            None => push_entry(&mut out, &key, &content),
            Some(p) => {
                // The key is `path#kind=value`; the error names the same.
                let key = String::from_utf8_lossy(&key);
                let path = &key[..key.len() - p.canonical().len() - 1];
                let slice = p
                    .apply(Path::new(path), &content)
                    .map_err(|e| hard(format!("inputs={path}#{e}")))?;
                push_entry(&mut out, key.as_bytes(), &slice);
            }
        }
    }
    Ok(out)
}

/// One snapshot entry: `path NUL length NUL content NUL`.
fn push_entry(out: &mut Vec<u8>, rel: &[u8], content: &[u8]) {
    out.extend_from_slice(rel);
    out.push(0);
    out.extend_from_slice(content.len().to_string().as_bytes());
    out.push(0);
    out.extend_from_slice(content);
    out.push(0);
}

/// Runs `cmd` under `/bin/sh -c` in the region root with the pinned
/// environment, stdin closed, in its own process group. When the shell
/// exits or the timeout expires, the group is killed: the output is what
/// the command printed before its shell was done, and a background job it
/// left behind cannot hold the pipes open. A process that left the group
/// and still holds them is a failure once the timeout has passed.
fn exec(ctx: &Ctx, args: &ExecArgs, region_name: &str) -> Result<String, LoadError> {
    let template = ctx
        .template
        .canonicalize()
        .unwrap_or_else(|_| ctx.template.clone());
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(&args.cmd)
        .current_dir(&ctx.region_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C")
        .env("LANGUAGE", "")
        .env("TZ", "UTC")
        .env("COMPUTED_FILE", &template)
        .env("COMPUTED_REGION", region_name);
    match &ctx.repo_root {
        Some(root) => command.env("COMPUTED_ROOT", root),
        None => command.env_remove("COMPUTED_ROOT"),
    };
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let deadline = Instant::now() + args.timeout;
    let mut child = command.spawn().map_err(|e| hard(format!("/bin/sh: {e}")))?;
    let (tx, rx) = mpsc::channel();
    let pipes: [Box<dyn Read + Send>; 2] = [
        Box::new(child.stdout.take().expect("piped")),
        Box::new(child.stderr.take().expect("piped")),
    ];
    for (i, mut pipe) in pipes.into_iter().enumerate() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = tx.send((i, buf));
        });
    }
    let status = wait_timeout::ChildExt::wait_timeout(&mut child, args.timeout)
        .map_err(|e| hard(format!("wait: {e}")))?;
    // SAFETY: kill(2) on the process group we created; the id is our child's.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let timed_out = status.is_none();
    if timed_out {
        let _ = child.wait();
    }
    let grace = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_secs(1));
    let until = Instant::now() + grace;
    let mut bufs: [Option<Vec<u8>>; 2] = [None, None];
    while bufs.iter().any(Option::is_none) {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok((i, buf)) => bufs[i] = Some(buf),
            Err(_) => break,
        }
    }
    let held = bufs.iter().any(Option::is_none);
    let [stdout, stderr] = bufs.map(Option::unwrap_or_default);
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    let failed = |reason: String| {
        let mut s = reason;
        if !stderr.is_empty() {
            s.push('\n');
            s.push_str(stderr.trim_end_matches('\n'));
        }
        LoadError::Failed { stderr: s }
    };
    if timed_out {
        return Err(failed(format!(
            "timed out after {}s",
            args.timeout.as_secs()
        )));
    }
    if held {
        return Err(failed(format!(
            "a process outside the command's process group kept its output open past {}s",
            args.timeout.as_secs()
        )));
    }
    let status = status.expect("not timed out");
    if !status.success() {
        return Err(failed(match status.code() {
            Some(c) => format!("exit status {c}"),
            None => "killed by a signal".to_string(),
        }));
    }
    String::from_utf8(stdout).map_err(|_| failed("stdout is not UTF-8".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::{self, Region, Segment};
    use crate::render::Loaders;
    use std::fs;
    use std::path::Path;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::create_dir_all(r.join("docs/adr")).unwrap();
        fs::create_dir_all(r.join("src")).unwrap();
        fs::create_dir_all(r.join("target")).unwrap();
        fs::write(r.join(".gitignore"), "target/\n").unwrap();
        fs::write(r.join("docs/adr/0001.md"), "# One\n").unwrap();
        fs::write(r.join("docs/adr/0002.md"), "# Two\n").unwrap();
        fs::write(r.join("docs/guide.md"), "").unwrap();
        fs::write(r.join("src/main.rs"), "").unwrap();
        fs::write(r.join("target/bin"), "").unwrap();
        fs::write(r.join("CLAUDE.md"), "").unwrap();
        dir
    }

    fn region(opener: &str) -> Region {
        let text = format!("{opener}\n<!-- /computed -->\n");
        match marker::parse(&text).unwrap().segments.remove(0) {
            Segment::Region(r) => r,
            Segment::Prose(_) => unreachable!(),
        }
    }

    fn ctx(root: &Path, template: &str) -> Ctx {
        let template = root.join(template);
        Ctx::for_template(&template)
    }

    #[test]
    fn tree_lists_and_snapshots_one_walk() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let r = region("<!-- computed tree src=. depth=2 name=layout -->");
        let snap = p.snapshot(&r).unwrap().unwrap();
        assert_eq!(
            String::from_utf8(snap.clone()).unwrap(),
            "CLAUDE.md\ndocs/\ndocs/adr/\ndocs/guide.md\nsrc/\nsrc/main.rs\n"
        );
        let loaded = p.load(&r).unwrap();
        assert_eq!(loaded.snapshot, snap);
        assert_eq!(
            loaded.text,
            ".\n├── CLAUDE.md\n├── docs\n│   ├── adr\n│   └── guide.md\n└── src\n    └── main.rs\n"
        );
    }

    #[test]
    fn tree_src_resolves_against_the_region_root_and_must_stay_inside_the_repository() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "docs/guide.md"));
        let r = region("<!-- computed tree src=adr dirs all -->");
        let loaded = p.load(&r).unwrap();
        assert_eq!(loaded.text, ".\n");
        let r = region("<!-- computed tree src=.. -->");
        assert!(
            p.snapshot(&r).is_ok(),
            "the repository root is inside the repository"
        );
        let r = region("<!-- computed tree src=../.. -->");
        assert!(matches!(p.snapshot(&r), Err(LoadError::Hard(m)) if m.contains("escapes")));
        let r = region("<!-- computed tree src=missing -->");
        assert!(matches!(p.snapshot(&r), Err(LoadError::Hard(_))));
    }

    #[test]
    fn exec_inputs_snapshot_lists_matched_files_with_their_content() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let r = region("<!-- computed exec cmd=true inputs=docs/adr/*.md,src -->");
        let snap = p.snapshot(&r).unwrap().unwrap();
        assert_eq!(snap, b"docs/adr/0001.md\x006\x00# One\n\x00docs/adr/0002.md\x006\x00# Two\n\x00src/main.rs\x000\x00\x00");
        let r = region("<!-- computed exec cmd=true inputs=**/*.md -->");
        let snap = String::from_utf8_lossy(&p.snapshot(&r).unwrap().unwrap()).to_string();
        assert!(
            !snap.contains("CLAUDE.md"),
            "the template is excluded from its own snapshot: {snap}"
        );
        assert!(snap.contains("docs/guide.md"));
        let r = region("<!-- computed exec cmd=true inputs=nothing/*.md -->");
        assert!(matches!(p.snapshot(&r), Err(LoadError::Hard(m)) if m.contains("matches nothing")));
        let r = region("<!-- computed exec cmd=true inputs=../*.md -->");
        assert!(matches!(p.snapshot(&r), Err(LoadError::Hard(m)) if m.contains("escapes")));
    }

    /// The relative paths a snapshot holds, in order.
    fn paths(mut snap: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        while !snap.is_empty() {
            let mut fields = snap.splitn(3, |&b| b == 0);
            let path = fields.next().unwrap();
            let len: usize = std::str::from_utf8(fields.next().unwrap())
                .unwrap()
                .parse()
                .unwrap();
            out.push(String::from_utf8(path.to_vec()).unwrap());
            snap = &fields.next().unwrap()[len + 1..];
        }
        out
    }

    fn inputs(p: &mut Production, globs: &str) -> Result<Vec<String>, LoadError> {
        let r = region(&format!("<!-- computed exec cmd=true inputs={globs} -->"));
        p.snapshot(&r).map(|s| paths(&s.unwrap()))
    }

    #[test]
    fn inputs_enter_only_directories_that_could_hold_a_match() {
        use std::os::unix::fs::PermissionsExt;
        let dir = repo();
        // A directory the walk cannot read stands in for a build churning
        // mid-walk: entering it fails, so pruning shows.
        let locked = dir.path().join("src/locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let observable = fs::read_dir(&locked).is_err();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let got = [
            ".gitignore",
            ".git*",
            "*.md",
            "docs/*/0001.md",
            "s*/main.rs",
            "**/*.md",
        ]
        .map(|glob| inputs(&mut p, glob));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if !observable {
            return;
        }
        for (i, snap) in got[..5].iter().enumerate() {
            assert!(snap.is_ok(), "glob {i}: {snap:?}");
        }
        assert!(
            matches!(&got[5], Err(LoadError::Hard(m)) if m.contains("src/locked")),
            "a `**` walk reads every directory, and its error names the one that failed: {:?}",
            got[5]
        );
    }

    #[test]
    fn wildcards_do_not_reach_ignored_paths_and_literal_components_do() {
        let dir = repo();
        let r = dir.path();
        fs::write(r.join(".gitignore"), "target/\n.claude/worktrees/\n").unwrap();
        fs::write(r.join("target/report.json"), "{}").unwrap();
        fs::create_dir_all(r.join(".claude/worktrees/w/docs")).unwrap();
        fs::write(r.join(".claude/worktrees/w/docs/guide.md"), "").unwrap();
        fs::write(r.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let mut p = Production::new(ctx(r, "CLAUDE.md"));
        let got = |p: &mut Production, globs| inputs(p, globs).unwrap();
        assert_eq!(
            got(&mut p, "**/*.md"),
            ["docs/adr/0001.md", "docs/adr/0002.md", "docs/guide.md"]
        );
        assert_eq!(
            got(&mut p, "**"),
            [
                ".gitignore",
                "docs/adr/0001.md",
                "docs/adr/0002.md",
                "docs/guide.md",
                "src/main.rs"
            ]
        );
        assert_eq!(got(&mut p, "target/*.json"), ["target/report.json"]);
        assert_eq!(got(&mut p, "**/target/*.json"), ["target/report.json"]);
        assert_eq!(got(&mut p, "target"), ["target/bin", "target/report.json"]);
        assert_eq!(got(&mut p, ".git/HEAD"), [".git/HEAD"]);
        assert_eq!(got(&mut p, ".git*"), [".gitignore"]);
        assert!(matches!(
            inputs(&mut p, "t*/bin"),
            Err(LoadError::Hard(m)) if m == "inputs=t*/bin matches nothing that is not ignored"
        ));
    }

    #[test]
    fn a_named_directory_takes_what_is_under_it_unless_ignored_there() {
        let dir = repo();
        let r = dir.path();
        fs::create_dir_all(r.join("notes/cache")).unwrap();
        fs::write(r.join("notes/.gitignore"), "*.md\n!keep.md\ncache/\n").unwrap();
        for f in [
            "notes/a.md",
            "notes/keep.md",
            "notes/b.txt",
            "notes/cache/c.txt",
        ] {
            fs::write(r.join(f), "").unwrap();
        }
        let mut p = Production::new(ctx(r, "CLAUDE.md"));
        assert_eq!(
            inputs(&mut p, "notes").unwrap(),
            ["notes/.gitignore", "notes/b.txt", "notes/keep.md"]
        );
        assert_eq!(inputs(&mut p, "notes/a.md").unwrap(), ["notes/a.md"]);
        assert_eq!(
            inputs(&mut p, "notes/cache/*").unwrap(),
            ["notes/cache/c.txt"]
        );
    }

    #[test]
    fn outside_a_repository_no_ignore_rules_apply() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("target")).unwrap();
        fs::write(r.join(".gitignore"), "target/\n").unwrap();
        fs::write(r.join("target/bin"), "").unwrap();
        let mut p = Production::new(ctx(r, "CLAUDE.md"));
        assert_eq!(inputs(&mut p, "*/*").unwrap(), ["target/bin"]);
    }

    /// The exec loader matches ignore rules path by path and the tree loader
    /// walks with the `ignore` crate; both must leave out the same files.
    #[test]
    fn a_wildcard_walk_selects_what_the_tree_walk_lists() {
        let dir = repo();
        let r = dir.path();
        fs::write(r.join(".gitignore"), "target/\n/build\n*.log\n!keep.log\n").unwrap();
        for d in ["build", "src/build", "docs/cache", "docs/adr/drafts"] {
            fs::create_dir_all(r.join(d)).unwrap();
        }
        fs::write(
            r.join("docs/.gitignore"),
            "cache/\n**/drafts/*.md\n!guide.md\n",
        )
        .unwrap();
        for f in [
            "build/out",
            "src/build/kept.rs",
            "a.log",
            "keep.log",
            "src/x.log",
            "docs/cache/c.md",
            "docs/adr/drafts/d.md",
            "docs/adr/drafts/d.txt",
            ".hidden",
        ] {
            fs::write(r.join(f), "").unwrap();
        }
        let listed: Vec<String> = crate::fs::walk(
            r,
            WalkOpts {
                all: true,
                ..WalkOpts::default()
            },
        )
        .filter(|e| !e.is_dir && e.path != Path::new("CLAUDE.md"))
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
        let mut p = Production::new(ctx(r, "CLAUDE.md"));
        let mut selected = inputs(&mut p, "**").unwrap();
        selected.sort();
        let mut listed = listed;
        listed.sort();
        assert_eq!(selected, listed);
        assert!(listed.contains(&"src/build/kept.rs".to_string()));
        assert!(!listed.contains(&"docs/adr/drafts/d.md".to_string()));
    }

    #[test]
    fn inputs_reach_outside_the_region_root_within_the_repository() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "docs/guide.md"));
        let r = region("<!-- computed exec cmd=true inputs=../src/*.rs,adr/0002.md -->");
        assert_eq!(
            p.snapshot(&r).unwrap().unwrap(),
            b"../src/main.rs\x000\x00\x00adr/0002.md\x006\x00# Two\n\x00"
        );
    }

    #[test]
    fn a_path_that_vanishes_during_expansion_is_skipped() {
        let dir = repo();
        let glob = InputGlob::new("**").unwrap();
        let bound = dir.path().canonicalize().unwrap();
        let mut expansion = Expansion::new(&glob, &bound, None);
        expansion
            .walk(
                &dir.path().join("gone"),
                Path::new("gone"),
                &Below::Everything,
                &Ignores::default(),
            )
            .unwrap();
        assert!(expansion.files.is_empty());
        let matched = BTreeMap::from([
            (b"CLAUDE.md".to_vec(), (dir.path().join("CLAUDE.md"), None)),
            (b"gone.md".to_vec(), (dir.path().join("gone.md"), None)),
        ]);
        assert_eq!(
            content_snapshot(matched, &mut BTreeSet::new()).unwrap(),
            b"CLAUDE.md\x000\x00\x00"
        );
    }

    #[test]
    fn exec_runs_in_the_region_root_with_the_pinned_environment() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "docs/guide.md"));
        let r = region(
            "<!-- computed exec cmd=\"pwd; echo $LC_ALL $TZ [$LANGUAGE] $COMPUTED_REGION; basename $COMPUTED_FILE; [ \\\"$COMPUTED_ROOT\\\" = \\\"$(cd .. && pwd -P)\\\" ] && echo root-ok\" volatile -->",
        );
        assert_eq!(p.snapshot(&r).unwrap(), None);
        let loaded = p.load(&r).unwrap();
        let expected_pwd = dir.path().join("docs").canonicalize().unwrap();
        let lines: Vec<&str> = loaded.text.lines().collect();
        assert_eq!(Path::new(lines[0]).canonicalize().unwrap(), expected_pwd);
        assert_eq!(lines[1], "C UTC [] exec@1");
        assert_eq!(lines[2], "guide.md");
        assert_eq!(lines[3], "root-ok");
        assert!(loaded.snapshot.is_empty());
        let r = region("<!-- computed exec cmd=\"echo $COMPUTED_REGION\" volatile name=n -->");
        assert_eq!(p.load(&r).unwrap().text, "n\n");
    }

    #[test]
    fn exec_failure_carries_stderr_and_a_timeout_kills_the_process_group() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let r = region("<!-- computed exec cmd=\"echo out; echo bad >&2; exit 3\" volatile -->");
        assert!(
            matches!(p.load(&r), Err(LoadError::Failed { stderr }) if stderr.contains("bad") && stderr.contains("exit"))
        );
        let r = region("<!-- computed exec cmd=\"sleep 5 & sleep 5\" volatile timeout=1 -->");
        let start = std::time::Instant::now();
        assert!(
            matches!(p.load(&r), Err(LoadError::Failed { stderr }) if stderr.contains("timed out"))
        );
        assert!(
            start.elapsed().as_secs() < 4,
            "the pipe closed once the group died"
        );
        let r = region("<!-- computed exec cmd=\"printf '\\377'\" volatile -->");
        assert!(
            matches!(p.load(&r), Err(LoadError::Failed { stderr }) if stderr.contains("UTF-8"))
        );
    }

    #[test]
    fn a_trailing_slash_names_the_directory() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        assert_eq!(
            inputs(&mut p, "docs/adr/").unwrap(),
            ["docs/adr/0001.md", "docs/adr/0002.md"]
        );
    }

    #[test]
    fn symlinks_inside_the_repository_are_read_through() {
        use std::os::unix::fs::symlink;
        let dir = repo();
        let r = dir.path();
        symlink("docs/adr", r.join("decisions")).unwrap();
        symlink("docs/adr/0001.md", r.join("first.md")).unwrap();
        symlink("adr/0001.md", r.join("docs/one.md")).unwrap();
        let mut p = Production::new(ctx(r, "CLAUDE.md"));
        assert_eq!(
            inputs(&mut p, "decisions/*.md").unwrap(),
            ["decisions/0001.md", "decisions/0002.md"]
        );
        assert_eq!(
            inputs(&mut p, "decisions").unwrap(),
            ["decisions/0001.md", "decisions/0002.md"]
        );
        assert_eq!(inputs(&mut p, "first.md").unwrap(), ["first.md"]);
        assert_eq!(
            inputs(&mut p, "docs/*.md").unwrap(),
            ["docs/guide.md", "docs/one.md"]
        );
        assert!(
            !inputs(&mut p, "*")
                .unwrap()
                .contains(&"decisions/0001.md".to_string()),
            "a wildcard does not enter a directory link"
        );
    }

    #[test]
    fn a_symlink_out_of_the_repository_is_an_error_when_named_and_skipped_otherwise() {
        use std::os::unix::fs::symlink;
        let dir = repo();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "x").unwrap();
        symlink(
            outside.path().join("secret.md"),
            dir.path().join("docs/out.md"),
        )
        .unwrap();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        assert_eq!(inputs(&mut p, "docs/*.md").unwrap(), ["docs/guide.md"]);
        assert!(
            matches!(inputs(&mut p, "docs/out.md"), Err(LoadError::Hard(m)) if m.contains("escapes")),
        );
    }

    #[test]
    fn snapshots_ignore_the_sums_in_other_templates() {
        let dir = repo();
        let sums = "in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715";
        let other = dir.path().join("docs/guide.md");
        fs::write(
            &other,
            format!("<!-- computed tree -->\n<!-- /computed {sums} -->\n"),
        )
        .unwrap();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let r = region("<!-- computed exec cmd=true inputs=docs/guide.md -->");
        let with_sums = p.snapshot(&r).unwrap();
        fs::write(&other, "<!-- computed tree -->\n<!-- /computed -->\n").unwrap();
        assert_eq!(p.snapshot(&r).unwrap(), with_sums);
        assert!(p.read().contains(&other.canonicalize().unwrap()));
    }

    #[test]
    fn file_includes_a_file_without_its_sums() {
        let dir = repo();
        let r = dir.path();
        fs::write(
            r.join("docs/shared.md"),
            "Shared.\n<!-- /computed in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715 -->\n",
        )
        .unwrap();
        let mut p = Production::new(ctx(r, "CLAUDE.md"));
        let region_ = region("<!-- computed file src=./docs/shared.md -->");
        let loaded = p.load(&region_).unwrap();
        assert_eq!(loaded.text, "Shared.\n<!-- /computed -->\n");
        assert_eq!(
            loaded.snapshot,
            b"docs/shared.md\x0027\x00Shared.\n<!-- /computed -->\n\x00"
        );
        assert_eq!(p.snapshot(&region_).unwrap(), Some(loaded.snapshot));
        for (opener, needle) in [
            ("<!-- computed file src=docs -->", "not a file"),
            ("<!-- computed file src=CLAUDE.md -->", "this file"),
            ("<!-- computed file src=missing.md -->", "missing.md"),
            ("<!-- computed file src=../x.md -->", ""),
        ] {
            assert!(
                matches!(p.snapshot(&region(opener)), Err(LoadError::Hard(m)) if m.contains(needle)),
                "{opener}"
            );
        }
    }

    #[test]
    fn tree_src_must_be_a_directory() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let r = region("<!-- computed tree src=src/main.rs -->");
        assert!(matches!(p.snapshot(&r), Err(LoadError::Hard(m)) if m.contains("not a directory")));
    }

    #[test]
    fn a_background_job_does_not_hold_the_region() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let r = region("<!-- computed exec cmd=\"(sleep 20 &); echo hi\" volatile timeout=10 -->");
        let start = std::time::Instant::now();
        assert_eq!(p.load(&r).unwrap().text, "hi\n");
        assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
    }

    #[test]
    fn stdin_is_closed() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let r = region("<!-- computed exec cmd=\"cat; echo done\" volatile -->");
        assert_eq!(p.load(&r).unwrap().text, "done\n");
    }
}
