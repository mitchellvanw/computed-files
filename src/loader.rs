//! The two loaders, `tree` and `exec`, and the production `Loaders` adapter.

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
        other => panic!("unknown loader {other:?} reached the format table"),
    }
}

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::fs::{self, WalkOpts};
use crate::marker::{Opener, Region};
use crate::render::Loaders;

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
                        return Err(hard("exec takes inputs= or volatile, not both"))
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
            other => Err(hard(format!("unknown loader {other:?}"))),
        }
    }

    pub fn format_constant(&self) -> u32 {
        match self {
            Loader::Tree(_) => format_constant("tree"),
            Loader::Exec(_) => format_constant("exec"),
        }
    }
}

/// The production `Loaders` adapter: resolves paths through a `Ctx` and
/// keeps each tree walk so `snapshot` and `load` cost one walk.
pub struct Production {
    ctx: Ctx,
    walks: HashMap<String, Loaded>,
}

impl Production {
    pub fn new(ctx: Ctx) -> Production {
        Production {
            ctx,
            walks: HashMap::new(),
        }
    }

    fn tree(&mut self, region: &Region, args: &TreeArgs) -> Result<Loaded, LoadError> {
        let key = region.opener.canonical();
        if let Some(l) = self.walks.get(&key) {
            return Ok(l.clone());
        }
        let src = self.ctx.resolve("src=", &args.src)?;
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
            }) => Ok(Some(inputs_snapshot(&self.ctx, &globs)?)),
        }
    }

    fn load(&mut self, region: &Region) -> Result<Loaded, LoadError> {
        match Loader::from_opener(&region.opener)? {
            Loader::Tree(args) => self.tree(region, &args),
            Loader::Exec(args) => {
                let snapshot = match &args.inputs {
                    None => Vec::new(),
                    Some(globs) => inputs_snapshot(&self.ctx, globs)?,
                };
                let text = exec(&self.ctx, &args, &self.region_name(region))?;
                Ok(Loaded { text, snapshot })
            }
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

/// Every file under `dir`, recursively, byte-order sorted, no ignore rules.
fn files_under(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let ft = e.file_type()?;
        if ft.is_dir() {
            files_under(&e.path(), out)?;
        } else if ft.is_file() {
            out.push(e.path());
        }
    }
    Ok(())
}

/// The literal directory a glob starts in, before its first wildcard.
fn glob_prefix(glob: &str) -> PathBuf {
    let mut prefix = PathBuf::new();
    for comp in glob.split('/') {
        if comp.contains(['*', '?', '[']) {
            break;
        }
        prefix.push(comp);
    }
    if glob.split('/').all(|c| !c.contains(['*', '?', '['])) {
        // A literal path names a file or directory; match from its parent.
        prefix.pop();
    }
    if prefix.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        prefix
    }
}

/// The `inputs=` snapshot: for every matched file in byte-order relative
/// path, `path NUL length NUL content NUL`. A matched directory means every
/// file under it; the template itself is excluded.
fn inputs_snapshot(ctx: &Ctx, globs: &[String]) -> Result<Vec<u8>, LoadError> {
    let template = ctx.template.canonicalize().ok();
    let mut matched: BTreeMap<Vec<u8>, PathBuf> = BTreeMap::new();
    for glob in globs {
        let glob = glob.trim();
        let matcher = globset::GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
            .map_err(|e| hard(format!("inputs={glob}: {e}")))?
            .compile_matcher();
        let prefix = glob_prefix(glob);
        if !ctx.region_root.join(&prefix).exists() {
            return Err(hard(format!("inputs={glob} matches nothing")));
        }
        let dir = ctx.resolve("inputs=", &prefix)?;
        let mut files = Vec::new();
        files_under(&dir, &mut files)
            .map_err(|e| hard(format!("inputs={glob}: {}: {e}", dir.display())))?;
        let region_root = ctx
            .region_root
            .canonicalize()
            .map_err(|e| hard(format!("region root: {e}")))?;
        let mut any = false;
        for file in files {
            let rel = relative(&region_root, &file);
            let hit = rel
                .ancestors()
                .any(|a| !a.as_os_str().is_empty() && matcher.is_match(a));
            if !hit {
                continue;
            }
            any = true;
            if template.as_ref() == Some(&file) {
                continue;
            }
            matched.insert(rel.to_string_lossy().as_bytes().to_vec(), file);
        }
        if !any {
            return Err(hard(format!("inputs={glob} matches nothing")));
        }
    }
    let mut out = Vec::new();
    for (rel, file) in matched {
        let content =
            std::fs::read(&file).map_err(|e| hard(format!("inputs: {}: {e}", file.display())))?;
        out.extend_from_slice(&rel);
        out.push(0);
        out.extend_from_slice(content.len().to_string().as_bytes());
        out.push(0);
        out.extend_from_slice(&content);
        out.push(0);
    }
    Ok(out)
}

/// `path` relative to `base`, using `..` where it lies outside.
fn relative(base: &Path, path: &Path) -> PathBuf {
    if let Ok(r) = path.strip_prefix(base) {
        return r.to_path_buf();
    }
    let mut up = PathBuf::new();
    let mut ancestor = base;
    loop {
        up.push("..");
        ancestor = ancestor.parent().unwrap_or(Path::new("/"));
        if let Ok(r) = path.strip_prefix(ancestor) {
            return up.join(r);
        }
        if ancestor == Path::new("/") {
            return path.to_path_buf();
        }
    }
}

/// Runs `cmd` under `/bin/sh -c` in the region root with the pinned
/// environment, stdin closed, in its own process group so a timeout kills
/// everything it started.
fn exec(ctx: &Ctx, args: &ExecArgs, region_name: &str) -> Result<String, LoadError> {
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
        .env("COMPUTED_FILE", &ctx.template)
        .env("COMPUTED_REGION", region_name);
    match &ctx.repo_root {
        Some(root) => command.env("COMPUTED_ROOT", root),
        None => command.env_remove("COMPUTED_ROOT"),
    };
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|e| hard(format!("/bin/sh: {e}")))?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let status = wait_timeout::ChildExt::wait_timeout(&mut child, args.timeout)
        .map_err(|e| hard(format!("wait: {e}")))?;
    let timed_out = status.is_none();
    if timed_out {
        // SAFETY: kill(2) on a process group we created; the id is our child's.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).into_owned();
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

    #[test]
    fn exec_runs_in_the_region_root_with_the_pinned_environment() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "docs/guide.md"));
        let r = region("<!-- computed exec cmd=\"pwd; echo $LC_ALL $TZ [$LANGUAGE] $COMPUTED_REGION; basename $COMPUTED_FILE; [ \\\"$COMPUTED_ROOT\\\" = \\\"$(cd .. && pwd -P)\\\" ] && echo root-ok\" volatile -->");
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
    fn stdin_is_closed() {
        let dir = repo();
        let mut p = Production::new(ctx(dir.path(), "CLAUDE.md"));
        let r = region("<!-- computed exec cmd=\"cat; echo done\" volatile -->");
        assert_eq!(p.load(&r).unwrap().text, "done\n");
    }
}
