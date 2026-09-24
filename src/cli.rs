//! The five commands: clap definitions, discovery, per-file context and
//! trust, the mapping from `Rendered` to a write and an exit tier, and the
//! passes that let templates reading each other settle in one `run`.
//! The only module using `anyhow`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use crate::loader::{Ctx, Production};
use crate::marker::{self, Region};
use crate::render::{self, Action, Mode, RegionReport, Rendered};
use crate::trust::{self, Store};
use crate::{fs, report};

#[derive(Parser)]
#[command(
    name = "computed",
    version,
    about = "Keeps marked regions of a markdown file current by computation"
)]
struct Cli {
    /// Show the regions that are otherwise silent.
    #[arg(short, long, global = true)]
    verbose: bool,
    /// Report as text on stderr, or as one JSON document on stdout.
    #[arg(long, global = true, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render every stale, unrendered or volatile region and write the files.
    Run {
        paths: Vec<PathBuf>,
        /// Overwrite hand-edited regions and re-render fresh ones.
        #[arg(long)]
        force: bool,
        /// Print a unified diff per file that would change; write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Treat every file as trusted for this invocation without writing the store.
        #[arg(long)]
        trust: bool,
        /// Only the regions with this name; repeat for more.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
    },
    /// Report every region's state without running a loader.
    Check {
        paths: Vec<PathBuf>,
        /// Only the regions with this name; repeat for more.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
    },
    /// Empty every region and strip its sums, leaving it unrendered.
    Clean {
        paths: Vec<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
        /// Only the regions with this name; repeat for more.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
    },
    /// Trust the repository containing PATH (default: the current directory).
    Trust { path: Option<PathBuf> },
    /// Remove the grant for the repository containing PATH.
    Untrust { path: Option<PathBuf> },
    /// Whether an edit changes a region's body or closer: the proposed text
    /// against FILE, or the Claude Code hook whose JSON is on stdin.
    Guard {
        #[arg(required_unless_present = "hook")]
        file: Option<PathBuf>,
        /// The file's text after the edit.
        #[arg(long, value_name = "PATH", required_unless_present = "hook")]
        proposed: Option<PathBuf>,
        /// Answer a Claude Code PreToolUse (`pre`) or PostToolUse (`post`) hook.
        #[arg(long, value_enum, conflicts_with_all = ["file", "proposed"])]
        hook: Option<crate::guard::Hook>,
    },
}

/// Runs the command line and returns the exit code.
pub fn main() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { 2 } else { 0 };
        }
    };
    match dispatch(cli) {
        Ok(tier) => i32::from(tier),
        Err(e) => {
            eprintln!("computed: {e:#}");
            2
        }
    }
}

/// What one invocation of `run`, `check` or `clean` asks for.
struct Job<'a> {
    mode: Mode,
    trust: bool,
    only: &'a [String],
    verbose: bool,
    format: Format,
}

fn dispatch(cli: Cli) -> Result<u8> {
    let job = |mode, trust, only| Job {
        mode,
        trust,
        only,
        verbose: cli.verbose,
        format: cli.format,
    };
    match &cli.command {
        Cmd::Run {
            paths,
            force,
            dry_run,
            trust,
            only,
        } => {
            let mode = if *dry_run {
                Mode::DryRun { force: *force }
            } else {
                Mode::Run { force: *force }
            };
            process(paths, &job(mode, *trust, only))
        }
        Cmd::Check { paths, only } => process(paths, &job(Mode::Check, false, only)),
        Cmd::Clean {
            paths,
            force,
            dry_run,
            only,
        } => {
            let mode = Mode::Clean {
                force: *force,
                dry_run: *dry_run,
            };
            process(paths, &job(mode, false, only))
        }
        Cmd::Trust { path } => {
            let root = trust::root_for(path.as_deref().unwrap_or(Path::new(".")))?;
            let recorded = Store::at(Store::default_path()?).grant(&root)?;
            println!("{}", recorded.display());
            Ok(0)
        }
        Cmd::Untrust { path } => {
            let root = trust::root_for(path.as_deref().unwrap_or(Path::new(".")))?;
            let store = Store::at(Store::default_path()?);
            if store.revoke(&root)? {
                println!("{}", root.display());
            } else {
                eprintln!("computed: {} was not trusted", root.display());
            }
            Ok(0)
        }
        Cmd::Guard {
            file,
            proposed,
            hook,
        } => Ok(crate::guard::command(
            file.as_deref(),
            proposed.as_deref(),
            *hook,
            cli.format == Format::Json,
        )?),
    }
}

/// Whether a walked file is a template candidate by its extension.
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e == "md" || e == "markdown")
}

/// Files to process, byte-order sorted: explicit files whatever their
/// extension, walked directories and the current directory for `.md` and
/// `.markdown`, dotfiles included, symlinks left to the files they name.
/// Two paths to one file are one file, named by the path that is not a link.
fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    };
    let walk = fs::WalkOpts {
        all: true,
        ..fs::WalkOpts::default()
    };
    for root in roots {
        let meta = std::fs::metadata(&root).with_context(|| format!("{}", root.display()))?;
        if meta.is_dir() {
            for entry in fs::walk(&root, walk) {
                if !entry.is_dir && !entry.is_link && is_markdown(&entry.path) {
                    let p = if root == Path::new(".") {
                        entry.path
                    } else {
                        root.join(entry.path)
                    };
                    files.push(p);
                }
            }
        } else {
            files.push(root);
        }
    }
    files.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
    files.dedup();
    let mut by_target: HashMap<PathBuf, usize> = HashMap::new();
    let mut kept: Vec<Option<PathBuf>> = Vec::new();
    for f in files {
        let target = f.canonicalize().unwrap_or_else(|_| f.clone());
        match by_target.get(&target) {
            Some(&i) if is_link(kept[i].as_deref().unwrap()) && !is_link(&f) => kept[i] = Some(f),
            Some(_) => {}
            None => {
                by_target.insert(target, kept.len());
                kept.push(Some(f));
            }
        }
    }
    Ok(kept.into_iter().flatten().collect())
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// What processing one file came to.
#[derive(Default)]
struct Outcome {
    tier: u8,
    regions: Vec<RegionReport>,
    /// A file-level error, at a line or on the file itself.
    error: Option<(Option<usize>, String)>,
    diff: Option<String>,
    /// The canonical path written, when the file was.
    written: Option<PathBuf>,
    /// The canonical files the file's snapshots read.
    read: BTreeSet<PathBuf>,
    /// Every region name in the file.
    names: Vec<String>,
}

impl Outcome {
    fn error(line: Option<usize>, message: impl Into<String>) -> Outcome {
        Outcome {
            tier: 2,
            error: Some((line, message.into())),
            ..Outcome::default()
        }
    }
}

/// Everything printed: text as each file finishes, each region block once
/// however many passes report it, or one JSON document at the end.
struct Printer {
    format: Format,
    mode: Mode,
    verbose: bool,
    printed: HashSet<String>,
    order: Vec<PathBuf>,
    json: HashMap<PathBuf, Outcome>,
}

impl Printer {
    fn file(&mut self, path: &Path, outcome: &Outcome) {
        if self.format == Format::Json {
            self.merge(path, outcome);
            return;
        }
        let mut err = std::io::stderr().lock();
        if let Some((line, message)) = &outcome.error {
            let _ = err.write_all(report::error(path, *line, message).as_bytes());
        }
        for block in report::regions(path, &outcome.regions, self.mode, self.verbose) {
            if self.printed.insert(block.clone()) {
                let _ = err.write_all(block.as_bytes());
            }
        }
        if let Some(diff) = &outcome.diff {
            print!("{diff}");
        }
    }

    /// A later pass's report of a region replaces an earlier one, unless it
    /// only says the region is now fresh: what was done to it stands.
    fn merge(&mut self, path: &Path, outcome: &Outcome) {
        let entry = self.json.entry(path.to_path_buf()).or_insert_with(|| {
            self.order.push(path.to_path_buf());
            Outcome::default()
        });
        entry.tier = entry.tier.max(outcome.tier);
        if outcome.error.is_some() {
            entry.error.clone_from(&outcome.error);
        }
        if outcome.diff.is_some() {
            entry.diff.clone_from(&outcome.diff);
        }
        for r in &outcome.regions {
            match entry.regions.iter_mut().find(|e| e.line == r.line) {
                Some(_) if r.action == Some(Action::Fresh) => {}
                Some(e) => *e = r.clone(),
                None => entry.regions.push(r.clone()),
            }
        }
    }

    fn message(&mut self, path: &Path, message: &str) {
        match self.format {
            Format::Text => eprint!("{}", report::error(path, None, message)),
            Format::Json => {
                let entry = Outcome::error(None, message);
                self.merge(path, &entry);
            }
        }
    }

    fn finish(&self, exit: u8) {
        if self.format != Format::Json {
            return;
        }
        let files: Vec<report::FileJson<'_>> = self
            .order
            .iter()
            .map(|p| {
                let o = &self.json[p];
                report::FileJson {
                    path: p,
                    error: o.error.as_ref().map(|(l, m)| (*l, m.as_str())),
                    regions: &o.regions,
                    diff: o.diff.as_deref(),
                }
            })
            .filter(|f| f.error.is_some() || !f.regions.is_empty() || f.diff.is_some())
            .collect();
        print!("{}", report::json(&files, exit));
    }
}

/// Processes every file, then, under `run`, again every file whose
/// snapshots read a file the last pass wrote, until a pass writes nothing.
/// A template that reads another is then fresh after one `run`, whatever
/// order the two sort in. Files that keep changing each other are an error
/// once every file has had a pass of its own.
fn process(paths: &[PathBuf], job: &Job<'_>) -> Result<u8> {
    let files = discover(paths)?;
    let store = Store::at(Store::default_path()?);
    let mut printer = Printer {
        format: job.format,
        mode: job.mode,
        verbose: job.verbose,
        printed: HashSet::new(),
        order: Vec::new(),
        json: HashMap::new(),
    };
    let mut tier = 0;
    let mut names: HashSet<String> = HashSet::new();
    let mut reads: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    let mut queue = files.clone();
    let settles = matches!(job.mode, Mode::Run { .. });
    for pass in 1.. {
        let mut written = BTreeSet::new();
        for path in &queue {
            let outcome = process_file(path, job, &store);
            printer.file(path, &outcome);
            tier = tier.max(outcome.tier);
            names.extend(outcome.names.iter().cloned());
            written.extend(outcome.written.clone());
            reads.insert(path.clone(), outcome.read);
        }
        if !settles || written.is_empty() {
            break;
        }
        queue = files
            .iter()
            .filter(|f| reads.get(*f).is_some_and(|r| !r.is_disjoint(&written)))
            .cloned()
            .collect();
        if queue.is_empty() {
            break;
        }
        if pass > files.len() {
            for path in &queue {
                printer.message(
                    path,
                    "still changing after a pass per file: its inputs include a file its own render changes",
                );
            }
            tier = 2;
            break;
        }
    }
    for name in job.only.iter().filter(|n| !names.contains(*n)) {
        match job.format {
            Format::Text => eprintln!("computed: no region is named {name:?}"),
            Format::Json => {
                printer.message(Path::new("."), &format!("no region is named {name:?}"))
            }
        }
        tier = 2;
    }
    printer.finish(tier);
    Ok(tier)
}

fn process_file(path: &Path, job: &Job<'_>, store: &Store) -> Outcome {
    match try_process_file(path, job, store) {
        Ok(outcome) => outcome,
        Err(e) => Outcome::error(None, format!("{e:#}")),
    }
}

fn try_process_file(path: &Path, job: &Job<'_>, store: &Store) -> Result<Outcome> {
    // A symlinked template is its target: paths resolve against the
    // target's directory and the write lands in the target.
    let file = if is_link(path) {
        path.canonicalize().context("unreadable")?
    } else {
        path.to_path_buf()
    };
    let bytes = std::fs::read(&file).context("unreadable")?;
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) if marker::has_marker(&String::from_utf8_lossy(e.as_bytes())) => {
            return Ok(Outcome::error(None, "not UTF-8"));
        }
        // A file with no marker is none of the tool's business, whatever its encoding.
        Err(_) => return Ok(Outcome::default()),
    };
    if !text.contains("<!--") {
        return Ok(Outcome::default());
    }
    let parsed = match marker::parse(&text) {
        Ok(p) => p,
        Err(e) => return Ok(Outcome::error(Some(e.line), e.message)),
    };
    let names: Vec<String> = parsed
        .segments
        .iter()
        .filter_map(|s| match s {
            marker::Segment::Region(r) => Some(r.opener.name.clone()),
            marker::Segment::Prose(_) => None,
        })
        .flatten()
        .collect();
    if !parsed
        .segments
        .iter()
        .any(|s| matches!(s, marker::Segment::Region(_)))
    {
        return Ok(Outcome::default());
    }
    let ctx = Ctx::for_template(&file);
    let needs_trust = matches!(job.mode, Mode::Run { .. } | Mode::DryRun { .. });
    let trusted = needs_trust
        && (job.trust || {
            let root = ctx
                .repo_root
                .clone()
                .map(Ok)
                .unwrap_or_else(|| trust::root_for(&ctx.region_root))?;
            store.is_trusted(&root)?
        });
    let select = |r: &Region| {
        job.only.is_empty() || r.opener.name.as_ref().is_some_and(|n| job.only.contains(n))
    };
    let mut loaders = Production::new(ctx);
    let rendered = render::file_where(&parsed, job.mode, trusted, &select, &mut loaders);
    let mut outcome = Outcome {
        tier: rendered.tier(),
        regions: rendered.regions().to_vec(),
        names,
        ..Outcome::default()
    };
    match &rendered {
        Rendered::Error { line, message } => {
            outcome.error = Some((Some(*line), message.clone()));
        }
        Rendered::Written { text: new, .. } => {
            if job.mode.dry_run() {
                outcome.diff = Some(report::diff(path, &text, new, None));
            } else {
                match fs::replace(&file, &text, new) {
                    Ok(true) => outcome.written = Some(file.canonicalize()?),
                    Ok(false) => {}
                    Err(e) => {
                        return Ok(Outcome::error(None, format!("writing: {e}")));
                    }
                }
            }
        }
        // The diff `--force` would apply, so the edit can be kept by hand.
        Rendered::Refused { .. } if job.mode == (Mode::DryRun { force: false }) => {
            let forced = Mode::DryRun { force: true };
            if let Rendered::Written { text: new, .. } =
                render::file_where(&parsed, forced, trusted, &select, &mut loaders)
            {
                outcome.diff = Some(report::diff(path, &text, &new, Some("run --force")));
            }
        }
        Rendered::Unchanged { .. } | Rendered::Refused { .. } => {}
    }
    outcome.read = loaders.read().clone();
    Ok(outcome)
}
