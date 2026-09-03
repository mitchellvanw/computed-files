//! The five commands: clap definitions, discovery, per-file context and
//! trust, and the mapping from `Rendered` to a write and an exit tier.
//! The only module using `anyhow`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::loader::{Ctx, Production};
use crate::marker;
use crate::render::{self, Mode, Rendered};
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
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render every stale, unrendered or volatile region and write the files.
    Run {
        paths: Vec<PathBuf>,
        /// Overwrite hand-edited regions.
        #[arg(long)]
        force: bool,
        /// Print a unified diff per file that would change; write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Treat every file as trusted for this invocation without writing the store.
        #[arg(long)]
        trust: bool,
    },
    /// Report every region's state without running a loader.
    Check { paths: Vec<PathBuf> },
    /// Empty every region and strip its sums, leaving it unrendered.
    Clean {
        paths: Vec<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Trust the repository containing PATH (default: the current directory).
    Trust { path: Option<PathBuf> },
    /// Remove the grant for the repository containing PATH.
    Untrust { path: Option<PathBuf> },
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

fn dispatch(cli: Cli) -> Result<u8> {
    match cli.command {
        Cmd::Run {
            paths,
            force,
            dry_run,
            trust,
        } => {
            let mode = if dry_run {
                Mode::DryRun { force }
            } else {
                Mode::Run { force }
            };
            process(&paths, mode, trust, cli.verbose)
        }
        Cmd::Check { paths } => process(&paths, Mode::Check, false, cli.verbose),
        Cmd::Clean {
            paths,
            force,
            dry_run,
        } => process(&paths, Mode::Clean { force, dry_run }, false, cli.verbose),
        Cmd::Trust { path } => {
            let root = trust::root_for(&path.unwrap_or_else(|| PathBuf::from(".")))?;
            let recorded = Store::at(Store::default_path()?).grant(&root)?;
            println!("{}", recorded.display());
            Ok(0)
        }
        Cmd::Untrust { path } => {
            let root = trust::root_for(&path.unwrap_or_else(|| PathBuf::from(".")))?;
            let store = Store::at(Store::default_path()?);
            if store.revoke(&root)? {
                println!("{}", root.display());
            } else {
                eprintln!("computed: {} was not trusted", root.display());
            }
            Ok(0)
        }
    }
}

/// Files to process, byte-order sorted: explicit files whatever their
/// extension, walked directories and the current directory for `.md` only.
fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    };
    for root in roots {
        let meta = std::fs::metadata(&root).with_context(|| format!("{}", root.display()))?;
        if meta.is_dir() {
            for entry in fs::walk(&root, fs::WalkOpts::default()) {
                if !entry.is_dir && entry.path.extension().is_some_and(|e| e == "md") {
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
    Ok(files)
}

fn process(paths: &[PathBuf], mode: Mode, trust_flag: bool, verbose: bool) -> Result<u8> {
    let files = discover(paths)?;
    let store = Store::at(Store::default_path()?);
    let mut tier = 0;
    let stderr = std::io::stderr();
    let stdout = std::io::stdout();
    for path in files {
        let file_tier = match process_file(
            &path,
            mode,
            trust_flag,
            &store,
            verbose,
            &mut stderr.lock(),
            &mut stdout.lock(),
        ) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}: {e:#}", path.display());
                2
            }
        };
        tier = tier.max(file_tier);
    }
    Ok(tier)
}

fn process_file(
    path: &Path,
    mode: Mode,
    trust_flag: bool,
    store: &Store,
    verbose: bool,
    err: &mut dyn std::io::Write,
    out: &mut dyn std::io::Write,
) -> Result<u8> {
    let text = std::fs::read_to_string(path).context("unreadable")?;
    if !text.contains("<!--") {
        return Ok(0);
    }
    let parsed = match marker::parse(&text) {
        Ok(p) => p,
        Err(e) => {
            write!(err, "{}", report::error(path, e.line, &e.message))?;
            return Ok(2);
        }
    };
    if !parsed
        .segments
        .iter()
        .any(|s| matches!(s, marker::Segment::Region(_)))
    {
        return Ok(0);
    }
    let ctx = Ctx::for_template(path);
    let needs_trust = matches!(mode, Mode::Run { .. } | Mode::DryRun { .. });
    let trusted = needs_trust
        && (trust_flag || {
            let root = ctx
                .repo_root
                .clone()
                .map(Ok)
                .unwrap_or_else(|| trust::root_for(&ctx.region_root))?;
            store.is_trusted(&root)?
        });
    let mut loaders = Production::new(ctx);
    let rendered = render::file(&parsed, mode, trusted, &mut loaders);
    match &rendered {
        Rendered::Error { line, message } => {
            write!(err, "{}", report::error(path, *line, message))?;
        }
        Rendered::Written { text: new, .. } => {
            if mode.dry_run() {
                write!(out, "{}", report::diff(path, &text, new))?;
            } else {
                fs::write(path, new).with_context(|| format!("writing {}", path.display()))?;
            }
            write!(err, "{}", report::regions(path, &rendered, mode, verbose))?;
        }
        Rendered::Unchanged { .. } | Rendered::Refused { .. } => {
            write!(err, "{}", report::regions(path, &rendered, mode, verbose))?;
        }
    }
    Ok(rendered.tier())
}
