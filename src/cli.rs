//! The commands: clap definitions, discovery, per-file context and trust,
//! the mapping from `Rendered` to a write and an exit tier, the passes that
//! let templates reading each other settle in one `run`, and one dispatch
//! arm for each command that lives in a module of its own.
//! The only module using `anyhow`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use crate::allow::{self, Allowed};
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
    /// `graph` only: a Mermaid flowchart, the default.
    Mermaid,
    /// `graph` only: a Graphviz digraph.
    Dot,
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
        /// Allow fetching under this url prefix for this invocation without writing the allowlist; repeat for more.
        #[arg(long, value_name = "PREFIX")]
        allow: Vec<String>,
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
    /// Fetch every remote region's url and pin its SHA-256 in the opener.
    Update {
        paths: Vec<PathBuf>,
        /// Print a unified diff per file that would change; write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Only the regions with this name; repeat for more.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
        /// Allow fetching under this url prefix for this invocation without writing the allowlist; repeat for more.
        #[arg(long, value_name = "PREFIX")]
        allow: Vec<String>,
    },
    /// Allow remote regions to fetch under PREFIX on this machine; with no PREFIX, list the allowlist.
    Allow { prefix: Option<String> },
    /// Remove PREFIX from the allowlist.
    Disallow { prefix: String },
    /// Trust the repository containing PATH (default: the current directory).
    Trust { path: Option<PathBuf> },
    /// Remove the grant for the repository containing PATH.
    Untrust { path: Option<PathBuf> },
    /// Count every region's lines, bytes and estimated tokens, and the share
    /// of each file that is computed, without running a loader.
    Stats { paths: Vec<PathBuf> },
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
    /// Run again whenever a template or a file its regions read changes.
    Watch {
        paths: Vec<PathBuf>,
        /// Treat every file as trusted for this invocation without writing the store.
        #[arg(long)]
        trust: bool,
        /// Allow fetching under this url prefix for this invocation without writing the allowlist; repeat for more.
        #[arg(long, value_name = "PREFIX")]
        allow: Vec<String>,
    },
    /// Serve the language server protocol on stdin and stdout.
    Lsp,
    /// List the regions whose snapshots read any of PATHS, or anything under them.
    Affected {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Print templates, their regions and what each reads as a graph.
    Graph { paths: Vec<PathBuf> },
    /// Explain from history why a region is stale.
    Why {
        file: PathBuf,
        /// Only the region with this name.
        #[arg(long, value_name = "NAME", conflicts_with = "line")]
        only: Option<String>,
        /// Only the region whose opener is on this line.
        #[arg(long, value_name = "N")]
        line: Option<usize>,
    },
    /// A merge driver: merge a template, leaving regions both sides re-rendered unrendered.
    Merge {
        /// Route Markdown through this driver in the repository's .gitattributes and this clone's config.
        #[arg(long, conflicts_with = "files")]
        install: bool,
        /// git's %O %A %B %P: the base, ours (written), theirs, and the path, unused.
        #[arg(
            value_names = ["BASE", "OURS", "THEIRS", "PATH"],
            num_args = 3..=4,
            required_unless_present = "install"
        )]
        files: Vec<PathBuf>,
    },
    /// Write a hand edit inside a file region back into its source, and render the region.
    Adopt {
        file: PathBuf,
        /// Only the regions with this name; repeat for more.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
        /// Print the diff each source would take; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Find verbatim blocks copied between Markdown files, outside regions.
    Dupes {
        paths: Vec<PathBuf>,
        /// The fewest lines with text of their own a block needs to count.
        #[arg(long, value_name = "N", default_value_t = 4)]
        min_lines: usize,
    },
    /// Run every loader twice, once under a perturbed environment, and report
    /// regions whose output differs or moved without their inputs. Writes nothing.
    Doctor {
        paths: Vec<PathBuf>,
        /// Treat every file as trusted for this invocation without writing the store.
        #[arg(long)]
        trust: bool,
        /// Allow fetching under this url prefix for this invocation without writing the allowlist; repeat for more.
        #[arg(long, value_name = "PREFIX")]
        allow: Vec<String>,
        /// Only the regions with this name; repeat for more.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
    },
    /// Trace each exec region's command and compare the files it reads with
    /// its inputs=.
    Trace {
        paths: Vec<PathBuf>,
        /// Treat every file as trusted for this invocation without writing the store.
        #[arg(long)]
        trust: bool,
        /// Only the regions with this name; repeat for more.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
        /// Rewrite each opener's inputs= with the suggestion.
        #[arg(long)]
        write: bool,
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
    /// The url prefixes `remote` regions may fetch under; `run` only.
    allowed: Option<&'a Allowed>,
}

fn dispatch(cli: Cli) -> Result<u8> {
    if matches!(cli.format, Format::Mermaid | Format::Dot)
        && !matches!(cli.command, Cmd::Graph { .. })
    {
        eprintln!("computed: --format mermaid and dot are for `graph`");
        return Ok(2);
    }
    let job = |mode, trust, only| Job {
        mode,
        trust,
        only,
        verbose: cli.verbose,
        format: cli.format,
        allowed: None,
    };
    match &cli.command {
        Cmd::Run {
            paths,
            force,
            dry_run,
            trust,
            only,
            allow,
        } => {
            let mode = if *dry_run {
                Mode::DryRun { force: *force }
            } else {
                Mode::Run { force: *force }
            };
            let allowed = allow::Store::at(allow::Store::default_path()?).allowed(allow)?;
            let job = Job {
                allowed: Some(&allowed),
                ..job(mode, *trust, only)
            };
            process(paths, &job)
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
        Cmd::Update {
            paths,
            dry_run,
            only,
            allow,
        } => {
            let allowed = allow::Store::at(allow::Store::default_path()?).allowed(allow)?;
            Ok(crate::update::run(
                &discover(paths)?,
                *dry_run,
                only,
                &allowed,
                cli.verbose,
                cli.format == Format::Json,
            ))
        }
        Cmd::Allow { prefix } => {
            let store = allow::Store::at(allow::Store::default_path()?);
            match prefix {
                Some(prefix) => println!("{}", store.allow(prefix)?),
                None => store.list()?.iter().for_each(|p| println!("{p}")),
            }
            Ok(0)
        }
        Cmd::Disallow { prefix } => {
            let store = allow::Store::at(allow::Store::default_path()?);
            match store.disallow(prefix)? {
                Some(removed) => println!("{removed}"),
                None => eprintln!("computed: {prefix} was not allowed"),
            }
            Ok(0)
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
        Cmd::Stats { paths } => Ok(crate::stats::run(
            &discover(paths)?,
            cli.format == Format::Json,
        )),
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
        Cmd::Watch {
            paths,
            trust,
            allow,
        } => {
            let store = allow::Store::at(allow::Store::default_path()?);
            // Read before the first pass, so a malformed allowlist stops
            // the watch, and again on every pass, as a run would read it.
            store.allowed(allow)?;
            let pass = || {
                let allowed = store.allowed(allow).map_err(|e| e.to_string())?;
                let job = Job {
                    allowed: Some(&allowed),
                    ..job(Mode::Run { force: false }, *trust, &[])
                };
                let s = settle(paths, &job).map_err(|e| format!("{e:#}"))?;
                Ok(crate::watch::Pass {
                    files: s.files,
                    read: s.read,
                    written: s.written,
                })
            };
            crate::watch::watch(paths, cli.format == Format::Text, pass).map_err(anyhow::Error::msg)
        }
        Cmd::Lsp => crate::lsp::main().map_err(anyhow::Error::msg),
        Cmd::Affected { paths } => {
            crate::affected::main(paths, cli.format == Format::Json).map_err(anyhow::Error::msg)
        }
        Cmd::Graph { paths } => {
            let style = match cli.format {
                Format::Json => crate::graph::Style::Json,
                Format::Dot => crate::graph::Style::Dot,
                Format::Text | Format::Mermaid => crate::graph::Style::Mermaid,
            };
            crate::graph::main(paths, style).map_err(anyhow::Error::msg)
        }
        Cmd::Why { file, only, line } => {
            text_only(cli.format, "why")?;
            crate::why::main(file, only.as_deref(), *line, cli.verbose).map_err(anyhow::Error::msg)
        }
        Cmd::Merge { install, files } => {
            text_only(cli.format, "merge")?;
            match files.as_slice() {
                _ if *install => crate::merge::install(),
                [base, ours, theirs, rest @ ..] => {
                    crate::merge::driver(base, ours, theirs, rest.first().map(PathBuf::as_path))
                }
                _ => unreachable!("clap requires three files or --install"),
            }
            .map_err(anyhow::Error::msg)
        }
        Cmd::Adopt {
            file,
            only,
            dry_run,
        } => {
            text_only(cli.format, "adopt")?;
            crate::adopt::main(file, only, *dry_run, cli.verbose).map_err(anyhow::Error::msg)
        }
        Cmd::Dupes { paths, min_lines } => {
            crate::dupes::main(paths, *min_lines, cli.format == Format::Json)
                .map_err(anyhow::Error::msg)
        }
        Cmd::Doctor {
            paths,
            trust,
            allow,
            only,
        } => crate::doctor::main(
            paths,
            &crate::doctor::Job {
                trust: *trust,
                only,
                allowed: &allow::Store::at(allow::Store::default_path()?).allowed(allow)?,
                verbose: cli.verbose,
                json: cli.format == Format::Json,
            },
        ),
        Cmd::Trace {
            paths,
            trust,
            only,
            write,
        } => crate::trace::main(
            paths,
            &crate::trace::Job {
                trust: *trust,
                only,
                write: *write,
                verbose: cli.verbose,
                json: cli.format == Format::Json,
            },
        ),
    }
}

/// A command that prints text only refuses `--format json`.
fn text_only(format: Format, command: &str) -> Result<()> {
    match format {
        Format::Text => Ok(()),
        _ => anyhow::bail!("`{command}` prints text only; --format is not for it"),
    }
}

/// Files to process, byte-order sorted: explicit files whatever their
/// extension, walked directories and the current directory for `.md` and
/// `.markdown`, dotfiles included, symlinks left to the files they name.
/// A walk also reads the extensions and names the `[discover]` table of
/// its repository root's `computed.toml` lists (the walked directory's,
/// outside a repository). Two paths to one file are one file, named by the
/// path that is not a link.
pub(crate) fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
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
            let top = fs::repo_root(&root).unwrap_or_else(|| root.clone());
            let wanted = crate::config::discovery(&top).map_err(anyhow::Error::msg)?;
            for entry in fs::walk(&root, walk) {
                if !entry.is_dir && !entry.is_link && wanted.selects(&entry.path) {
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

pub(crate) fn is_link(path: &Path) -> bool {
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
            match entry
                .regions
                .iter_mut()
                .find(|e| (e.line, e.column) == (r.line, r.column))
            {
                Some(_) if r.action == Some(Action::Fresh) => {}
                Some(e) => *e = r.clone(),
                None => entry.regions.push(r.clone()),
            }
        }
    }

    fn message(&mut self, path: &Path, message: &str) {
        match self.format {
            Format::Text | Format::Mermaid | Format::Dot => {
                eprint!("{}", report::error(path, None, message))
            }
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
    Ok(settle(paths, job)?.tier)
}

/// What [`process`] came to, which `watch` re-derives what it watches from.
struct Settled {
    tier: u8,
    files: Vec<PathBuf>,
    /// The canonical files every template's snapshots read.
    read: BTreeSet<PathBuf>,
    /// The canonical files written, over every pass.
    written: BTreeSet<PathBuf>,
}

fn settle(paths: &[PathBuf], job: &Job<'_>) -> Result<Settled> {
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
    let mut all_written = BTreeSet::new();
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
        all_written.extend(written.iter().cloned());
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
        // A template whose toc lists a heading with a region inside it
        // reads itself, and takes a second pass of its own.
        if pass > files.len() + 1 {
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
            Format::Text | Format::Mermaid | Format::Dot => {
                eprintln!("computed: no region is named {name:?}")
            }
            Format::Json => {
                printer.message(Path::new("."), &format!("no region is named {name:?}"))
            }
        }
        tier = 2;
    }
    printer.finish(tier);
    Ok(Settled {
        tier,
        read: reads.values().flatten().cloned().collect(),
        files,
        written: all_written,
    })
}

/// A template as `open` found it.
pub(crate) enum Opened {
    /// No region: none of the tool's business.
    Skip,
    /// Tier 2 for the file: not UTF-8, or a parse error at a line.
    Error(Option<usize>, String),
    Template {
        /// The file itself, a symlink resolved to its target.
        file: PathBuf,
        text: String,
        /// The parse, `use` regions expanded to their recipes' openers.
        parsed: marker::File,
        /// What expanding them came to, for [`Production::with_recipes`].
        recipes: crate::config::Expansion,
    },
}

/// Reads and parses one template and expands its recipes. A symlinked
/// template is its target: paths resolve against the target's directory
/// and a write lands in it.
pub(crate) fn open(path: &Path) -> Result<Opened> {
    let file = if is_link(path) {
        path.canonicalize().context("unreadable")?
    } else {
        path.to_path_buf()
    };
    let bytes = std::fs::read(&file).context("unreadable")?;
    let syntax = marker::Syntax::for_path(&file);
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) if marker::has_marker(&String::from_utf8_lossy(e.as_bytes()), syntax) => {
            return Ok(Opened::Error(None, "not UTF-8".to_string()));
        }
        // A file with no marker is none of the tool's business, whatever its encoding.
        Err(_) => return Ok(Opened::Skip),
    };
    if !syntax.may_hold(&text) {
        return Ok(Opened::Skip);
    }
    let mut parsed = match marker::parse_as(&text, syntax) {
        Ok(p) => p,
        Err(e) => return Ok(Opened::Error(Some(e.line), e.message)),
    };
    if !parsed
        .segments
        .iter()
        .any(|s| matches!(s, marker::Segment::Region(_)))
    {
        return Ok(Opened::Skip);
    }
    let ctx = Ctx::for_template(&file);
    let recipes = crate::config::expand(&mut parsed, &ctx.region_root, ctx.repo_root.as_deref());
    Ok(Opened::Template {
        file,
        text,
        parsed,
        recipes,
    })
}

/// Whether the store trusts the template's repository root, or its region
/// root outside a repository.
pub(crate) fn is_trusted(ctx: &Ctx, store: &Store) -> Result<bool> {
    let root = ctx
        .repo_root
        .clone()
        .map(Ok)
        .unwrap_or_else(|| trust::root_for(&ctx.region_root))?;
    Ok(store.is_trusted(&root)?)
}

fn process_file(path: &Path, job: &Job<'_>, store: &Store) -> Outcome {
    match try_process_file(path, job, store) {
        Ok(outcome) => outcome,
        Err(e) => Outcome::error(None, format!("{e:#}")),
    }
}

fn try_process_file(path: &Path, job: &Job<'_>, store: &Store) -> Result<Outcome> {
    let (file, text, parsed, recipes) = match open(path)? {
        Opened::Skip => return Ok(Outcome::default()),
        Opened::Error(line, message) => return Ok(Outcome::error(line, message)),
        Opened::Template {
            file,
            text,
            parsed,
            recipes,
        } => (file, text, parsed, recipes),
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
    let ctx = Ctx::for_template(&file);
    let needs_trust = matches!(job.mode, Mode::Run { .. } | Mode::DryRun { .. });
    let trusted = needs_trust && (job.trust || is_trusted(&ctx, store)?);
    let select = |r: &Region| {
        job.only.is_empty() || r.opener.name.as_ref().is_some_and(|n| job.only.contains(n))
    };
    let mut loaders = Production::new(ctx)
        .allowing(job.allowed.cloned().unwrap_or_default())
        .with_recipes(&recipes);
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
