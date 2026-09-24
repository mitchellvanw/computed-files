//! `computed trace`: runs each exec region's command under a file-access
//! tracer and compares the repository files it opened for reading with the
//! files its `inputs=` expands to. A read that is not declared is a change
//! `check` will not notice; a declared input that is not read makes the
//! region stale for nothing. Suggests an `inputs=` value, and with `--write`
//! puts it in the opener, which makes the region stale.
//!
//! Two tracers sit behind [`Tracer`]. On Linux, `strace`. On macOS,
//! `sandbox-exec` with a profile that allows everything and reports every
//! read under the repository root to the unified log, read back with
//! `log show`; neither needs root.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::cli::{self, Opened};
use crate::launch::{self, Wrap};
use crate::loader::{self, Ctx, ExecArgs, LoadError, Loader};
use crate::marker::{self, File, Region, Segment};
use crate::transcript::{self, TranscriptArgs, Workdir};
use crate::trust::Store;
use crate::{fs, report};

/// What a traced run came to: the command's text or its failure, and every
/// path it opened for reading, absolute, wherever it lies.
pub struct Traced {
    pub text: Result<String, LoadError>,
    pub reads: BTreeSet<PathBuf>,
}

/// What trace runs: an exec region's command or a transcript's steps,
/// always unsandboxed, since the reads trace looks for are the ones a
/// sandbox refuses.
#[derive(Debug, Clone)]
pub enum Traceable {
    Exec(ExecArgs),
    Transcript(TranscriptArgs),
}

impl Traceable {
    /// Runs it with `wrap` around its shell.
    pub fn run(&self, ctx: &Ctx, region: &str, wrap: &Wrap) -> Result<String, LoadError> {
        match self {
            Traceable::Exec(args) => loader::exec(ctx, args, region, Some(wrap)),
            Traceable::Transcript(args) => transcript::run(ctx, args, region, Some(wrap)),
        }
    }

    fn inputs(&self) -> Option<&Vec<String>> {
        match self {
            Traceable::Exec(args) => args.inputs.as_ref(),
            Traceable::Transcript(args) => args.inputs.as_ref(),
        }
    }
}

/// Runs a region's shell and records what it read.
pub trait Tracer {
    /// `Err` when the tracer itself failed: the tool cannot answer.
    fn trace(&mut self, ctx: &Ctx, job: &Traceable, region: &str) -> Result<Traced, String>;
}

/// The tracer this platform has, or why there is none.
pub fn detect() -> Result<Box<dyn Tracer>, String> {
    if cfg!(target_os = "macos") {
        Seatbelt::detect().map(|t| Box::new(t) as Box<dyn Tracer>)
    } else if cfg!(target_os = "linux") {
        Strace::detect().map(|t| Box::new(t) as Box<dyn Tracer>)
    } else {
        Err("trace needs strace on Linux or sandbox-exec on macOS".to_string())
    }
}

/// How a region's reads compare with its declared inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// It read exactly what it declares.
    Complete,
    /// It read a file it does not declare: `check` misses a change to it.
    Undeclared,
    /// It declares a file it does not read: a change to it restales for nothing.
    Unused,
    UndeclaredUnused,
    /// It declares no inputs; what it read is shown, not judged.
    Volatile,
    Failed,
    Untrusted,
    Error,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Verdict::Complete => "complete",
            Verdict::Undeclared => "undeclared",
            Verdict::Unused => "unused",
            Verdict::UndeclaredUnused => "undeclared+unused",
            Verdict::Volatile => "volatile",
            Verdict::Failed => "failed",
            Verdict::Untrusted => "untrusted",
            Verdict::Error => "error",
        })
    }
}

/// One exec or transcript region's trace. Paths are relative to the
/// region root, as `inputs=` spells them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    /// The opener's column for a region inside a line.
    pub column: Option<usize>,
    pub name: Option<String>,
    pub loader: String,
    pub verdict: Verdict,
    /// Every repository file the command read.
    pub reads: Vec<String>,
    pub undeclared: Vec<String>,
    pub unused: Vec<String>,
    /// The `inputs=` value that declares what was read.
    pub suggestion: Option<String>,
    /// `--write` put the suggestion in the opener.
    pub rewritten: bool,
    /// Loader stderr, or why the tool could not answer.
    pub message: Option<String>,
}

impl Finding {
    /// The exit tier this region contributes.
    pub fn tier(&self) -> u8 {
        match self.verdict {
            Verdict::Error => 2,
            Verdict::Undeclared
            | Verdict::UndeclaredUnused
            | Verdict::Failed
            | Verdict::Untrusted => 1,
            Verdict::Unused if self.rewritten => 1,
            Verdict::Complete | Verdict::Unused | Verdict::Volatile => 0,
        }
    }
}

/// Traces one exec region. `Err` only when the tracer failed.
pub fn examine(
    ctx: &Ctx,
    region: &Region,
    trusted: bool,
    tracer: &mut dyn Tracer,
) -> Result<Finding, String> {
    let mut finding = Finding {
        line: region.line,
        column: region.column,
        name: region.opener.name.clone(),
        loader: region.opener.loader.clone(),
        verdict: Verdict::Complete,
        reads: Vec::new(),
        undeclared: Vec::new(),
        unused: Vec::new(),
        suggestion: None,
        rewritten: false,
        message: None,
    };
    let error = |mut f: Finding, verdict, message| {
        f.verdict = verdict;
        f.message = Some(message);
        Ok(f)
    };
    // Unsandboxed: the reads trace looks for are the ones a sandbox refuses.
    let job = match Loader::from_opener(&region.opener) {
        Ok(Loader::Exec(args)) => Traceable::Exec(ExecArgs {
            sandbox: false,
            ..args
        }),
        Ok(Loader::Transcript(args)) if args.workdir == Workdir::Copy => {
            return error(
                finding,
                Verdict::Error,
                "workdir=copy: the steps read a copy of the repository, which trace cannot map back to its files".to_string(),
            );
        }
        Ok(Loader::Transcript(args)) => Traceable::Transcript(TranscriptArgs {
            sandbox: false,
            ..args
        }),
        Ok(_) => unreachable!("only exec and transcript regions are traced"),
        Err(LoadError::Hard(m) | LoadError::Failed { stderr: m } | LoadError::NotAllowed(m)) => {
            return error(finding, Verdict::Error, m);
        }
    };
    if !trusted {
        finding.verdict = Verdict::Untrusted;
        return Ok(finding);
    }
    let declared = match job.inputs() {
        None => None,
        Some(globs) => match loader::input_files(ctx, globs) {
            Ok(files) => Some(files),
            Err(
                LoadError::Hard(m) | LoadError::Failed { stderr: m } | LoadError::NotAllowed(m),
            ) => {
                return error(finding, Verdict::Error, m);
            }
        },
    };
    let name = region
        .opener
        .name
        .clone()
        .unwrap_or_else(|| format!("{}@{}", region.opener.loader, region.line));
    let traced = tracer.trace(ctx, &job, &name)?;
    match traced.text {
        Ok(_) => {}
        Err(LoadError::Failed { stderr }) => return error(finding, Verdict::Failed, stderr),
        Err(LoadError::Hard(m) | LoadError::NotAllowed(m)) => {
            return error(finding, Verdict::Error, m);
        }
    }
    let root = ctx.region_root.canonicalize().map_err(|e| e.to_string())?;
    let reads = relevant(ctx, &traced.reads)?;
    let spell = |p: &Path| relative(&root, p).to_string_lossy().into_owned();
    finding.reads = reads.iter().map(|p| spell(p)).collect();
    finding.suggestion = suggest(ctx, &reads);
    let Some(declared) = declared else {
        finding.verdict = Verdict::Volatile;
        return Ok(finding);
    };
    // A projected entry the command read stays as written: the projection
    // is the author's choice of what part matters, which a read cannot say.
    let projected: BTreeMap<&PathBuf, String> = declared
        .iter()
        .filter(|(key, file)| key.contains(&b'#') && reads.contains(*file))
        .map(|(key, file)| (file, String::from_utf8_lossy(key).into_owned()))
        .collect();
    if !projected.is_empty() {
        let rest: BTreeSet<PathBuf> = reads
            .iter()
            .filter(|p| !projected.contains_key(p))
            .cloned()
            .collect();
        let mut entries: Vec<String> = projected.values().cloned().collect();
        entries.extend(suggest(ctx, &rest));
        entries.sort();
        finding.suggestion = Some(entries.join(","));
    }
    let wanted: BTreeSet<&PathBuf> = declared.values().collect();
    finding.undeclared = reads
        .iter()
        .filter(|p| !wanted.contains(p))
        .map(|p| spell(p))
        .collect();
    finding.unused = declared
        .iter()
        .filter(|(_, p)| !reads.contains(*p))
        .map(|(rel, _)| String::from_utf8_lossy(rel).into_owned())
        .collect();
    finding.verdict = match (finding.undeclared.is_empty(), finding.unused.is_empty()) {
        (true, true) => Verdict::Complete,
        (false, true) => Verdict::Undeclared,
        (true, false) => Verdict::Unused,
        (false, false) => Verdict::UndeclaredUnused,
    };
    Ok(finding)
}

/// The reads that can be inputs: files that exist inside the repository
/// root (the region root outside one), not under `.git`, not the template.
fn relevant(ctx: &Ctx, reads: &BTreeSet<PathBuf>) -> Result<BTreeSet<PathBuf>, String> {
    let bound = ctx.bound().map_err(|e| match e {
        LoadError::Hard(m) | LoadError::Failed { stderr: m } | LoadError::NotAllowed(m) => m,
    })?;
    let template = ctx.template.canonicalize().ok();
    Ok(reads
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .filter(|p| p.starts_with(&bound) && p.is_file() && Some(p) != template.as_ref())
        .filter(|p| !p.components().any(|c| c.as_os_str() == ".git"))
        .collect())
}

/// `to` spelled from `from`, with `..` where it leaves it. Both absolute.
fn relative(from: &Path, to: &Path) -> PathBuf {
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

/// An `inputs=` value that selects exactly `reads`, byte-order sorted. Two
/// or more files of one directory with one extension become `dir/*.ext`
/// when that selects exactly them; with several extensions, `dir/*` when
/// that does, else `dir/*.ext` per extension that does. The rest stay
/// literal.
/// `None` when nothing was read.
pub fn suggest(ctx: &Ctx, reads: &BTreeSet<PathBuf>) -> Option<String> {
    let root = ctx.region_root.canonicalize().ok()?;
    let mut by_dir: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    for p in reads {
        let rel = relative(&root, p);
        let dir = rel.parent().map(Path::to_path_buf).unwrap_or_default();
        by_dir.entry(dir).or_default().insert(p.clone());
    }
    let selects = |glob: &str, files: &BTreeSet<PathBuf>| {
        loader::input_files(ctx, &[glob.to_string()])
            .is_ok_and(|m| m.values().cloned().collect::<BTreeSet<_>>() == *files)
    };
    let in_dir = |dir: &Path, pattern: &str| {
        if dir.as_os_str().is_empty() {
            pattern.to_string()
        } else {
            format!("{}/{pattern}", dir.to_string_lossy())
        }
    };
    let mut entries = Vec::new();
    for (dir, files) in by_dir {
        let exts: BTreeSet<_> = files.iter().map(|f| f.extension()).collect();
        let all = in_dir(&dir, "*");
        if files.len() > 1 && exts.len() > 1 && selects(&all, &files) {
            entries.push(all);
            continue;
        }
        let mut by_ext: BTreeMap<OsString, BTreeSet<PathBuf>> = BTreeMap::new();
        let mut literal = Vec::new();
        for f in &files {
            match f.extension() {
                Some(ext) => {
                    by_ext
                        .entry(ext.to_os_string())
                        .or_default()
                        .insert(f.clone());
                }
                None => literal.push(f.clone()),
            }
        }
        for (ext, group) in by_ext {
            let glob = in_dir(&dir, &format!("*.{}", ext.to_string_lossy()));
            if group.len() > 1 && selects(&glob, &group) {
                entries.push(glob);
            } else {
                literal.extend(group);
            }
        }
        entries.extend(
            literal
                .iter()
                .map(|f| relative(&root, f).to_string_lossy().into_owned()),
        );
    }
    entries.sort();
    (!entries.is_empty()).then(|| entries.join(","))
}

/// The file with `inputs=` of the region at each place (`Region::at`)
/// replaced. A rendered opener keeps its suffix; line endings and
/// indentation are kept.
pub fn rewrite(parsed: &File, inputs: &BTreeMap<(usize, Option<usize>), String>) -> String {
    let mut file = parsed.clone();
    for segment in &mut file.segments {
        let Segment::Region(r) = segment else {
            continue;
        };
        let Some(value) = inputs.get(&r.at()) else {
            continue;
        };
        let opener = r.opener.with_attr("inputs", value);
        let line = if r.raw_opener.contains(marker::OPENER_SUFFIX) {
            marker::rendered_opener(&opener, r.comment)
        } else {
            marker::opener_line(&opener, r.comment)
        };
        let body = r.raw_opener.trim_end_matches(['\n', '\r']);
        let eol = &r.raw_opener[body.len()..];
        r.raw_opener = format!("{}{line}{eol}", r.indent);
        r.opener = opener;
    }
    marker::serialise(&file)
}

/// Linux: `strace`, one output file per process, file descriptors decoded
/// to the paths they name.
struct Strace {
    program: PathBuf,
}

impl Strace {
    fn detect() -> Result<Strace, String> {
        let program = std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|d| d.join("strace"))
                    .find(|p| p.is_file())
            })
            .ok_or_else(|| {
                "trace needs strace on Linux and none is on PATH; install it (apt install strace, dnf install strace, apk add strace)".to_string()
            })?;
        // ptrace may be forbidden, as in a container without CAP_SYS_PTRACE.
        let probe = Command::new(&program)
            .args(["-qq", "-o", "/dev/null", "/bin/true"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{}: {e}", program.display()))?;
        if !probe.status.success() {
            return Err(format!(
                "{} cannot trace here: {}",
                program.display(),
                String::from_utf8_lossy(&probe.stderr).trim()
            ));
        }
        Ok(Strace { program })
    }
}

impl Tracer for Strace {
    fn trace(&mut self, ctx: &Ctx, job: &Traceable, region: &str) -> Result<Traced, String> {
        let dir = tempfile::tempdir().map_err(|e| format!("trace directory: {e}"))?;
        let out = dir.path().join("trace");
        let program = self.program.clone();
        let wrap = move |command: Command| {
            let mut prefix: Vec<OsString> = vec![program.clone().into()];
            prefix.extend(
                [
                    "-ff",
                    "-qq",
                    "-y",
                    "-s",
                    "4096",
                    "-e",
                    "trace=?open,openat,?openat2,execve,?execveat",
                    "-o",
                ]
                .map(OsString::from),
            );
            prefix.push(out.clone().into());
            prefix.push("--".into());
            Ok(launch::prefixed(&command, prefix))
        };
        let text = job.run(ctx, region, &wrap);
        let cwd = ctx.region_root.canonicalize().map_err(|e| e.to_string())?;
        let mut reads = BTreeSet::new();
        let mut traced = false;
        for entry in std::fs::read_dir(dir.path()).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let log = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            reads.extend(strace_reads(&String::from_utf8_lossy(&log), &cwd));
            traced = true;
        }
        if !traced {
            let why = match &text {
                Err(
                    LoadError::Failed { stderr }
                    | LoadError::Hard(stderr)
                    | LoadError::NotAllowed(stderr),
                ) => stderr.clone(),
                Ok(_) => String::new(),
            };
            return Err(format!("strace wrote no trace: {why}"));
        }
        Ok(Traced { text, reads })
    }
}

/// The paths an `strace -y` log shows opened for reading, or executed. A
/// successful open's return value names its file; an executed relative
/// path is taken against `cwd`, where the command started.
fn strace_reads(log: &str, cwd: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in log.lines() {
        let Some((call, rest)) = line.split_once('(') else {
            continue;
        };
        let Some((_, ret)) = rest.rsplit_once(") = ") else {
            continue;
        };
        if ret.starts_with('-') {
            continue;
        }
        match call {
            "open" | "openat" | "openat2" => {
                if !(rest.contains("O_RDONLY") || rest.contains("O_RDWR")) {
                    continue;
                }
                let decorated = ret
                    .trim_start_matches(|c: char| c.is_ascii_digit())
                    .strip_prefix('<')
                    .and_then(|r| r.trim_end().strip_suffix('>'));
                match decorated {
                    Some(p) => out.push(PathBuf::from(unescape(p))),
                    None => {
                        if let Some(p) = first_string(rest).filter(|p| p.starts_with('/')) {
                            out.push(PathBuf::from(p));
                        }
                    }
                }
            }
            "execve" | "execveat" => {
                if let Some(p) = first_string(rest) {
                    out.push(cwd.join(p));
                }
            }
            _ => {}
        }
    }
    out
}

/// The first double-quoted argument, unescaped.
fn first_string(args: &str) -> Option<String> {
    let start = args.find('"')? + 1;
    let mut end = start;
    let bytes = args.as_bytes();
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => end += 2,
            b'"' => return Some(unescape(&args[start..end])),
            _ => end += 1,
        }
    }
    None
}

/// strace's C escapes: `\n`, `\t`, `\"`, `\\`, `\xHH` and octal `\NNN`.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 == b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'v' => out.push(0x0b),
            b'f' => out.push(0x0c),
            b'x' => {
                let hex = s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'x'),
                }
            }
            b'0'..=b'7' => {
                let mut v = u32::from(c - b'0');
                let mut n = 1;
                while n < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                    v = v * 8 + u32::from(b[i] - b'0');
                    i += 1;
                    n += 1;
                }
                out.push(v as u8);
            }
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
const LOG: &str = "/usr/bin/log";

/// macOS: `sandbox-exec` with a profile that allows everything and reports
/// every read under the repository root to the unified log. `log stream`
/// does not show these reports to an unprivileged user, `log show` does, so
/// the log is read back after the command, until a sentinel read made
/// after it appears.
struct Seatbelt;

impl Seatbelt {
    fn detect() -> Result<Seatbelt, String> {
        for tool in [SANDBOX_EXEC, LOG] {
            if !Path::new(tool).is_file() {
                return Err(format!(
                    "trace needs {tool}, which this macOS does not have"
                ));
            }
        }
        Ok(Seatbelt)
    }
}

/// How long the unified log may take to show a trace's reports.
const LOG_WAIT: Duration = Duration::from_secs(30);

impl Tracer for Seatbelt {
    fn trace(&mut self, ctx: &Ctx, job: &Traceable, region: &str) -> Result<Traced, String> {
        let root = ctx.bound().map_err(|e| match e {
            LoadError::Hard(m) | LoadError::Failed { stderr: m } | LoadError::NotAllowed(m) => m,
        })?;
        let profile = reporting(&format!("(subpath {})", sbpl_string(&root)));
        let started = Instant::now();
        let first = Sentinel::read()?;
        let wrap =
            move |command: Command| Ok(launch::prefixed(&command, [SANDBOX_EXEC, "-p", &profile]));
        let text = job.run(ctx, region, &wrap);
        let last = Sentinel::read()?;
        let predicate = format!(
            "subsystem == \"com.apple.sandbox.reporting\" AND (eventMessage CONTAINS {} OR eventMessage CONTAINS {} OR eventMessage CONTAINS {})",
            predicate_string(&root),
            predicate_string(&first.path),
            predicate_string(&last.path)
        );
        let deadline = Instant::now() + LOG_WAIT;
        loop {
            let window = format!("{}s", started.elapsed().as_secs() + 5);
            let out = Command::new(LOG)
                .args([
                    "show",
                    "--style",
                    "compact",
                    "--last",
                    &window,
                    "--predicate",
                ])
                .arg(&predicate)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| format!("{LOG}: {e}"))?;
            let seen = sandbox_reads(&String::from_utf8_lossy(&out.stdout));
            let at = |s: &Sentinel| seen.iter().position(|p| *p == s.path);
            if let (Some(from), Some(to)) = (at(&first), at(&last)) {
                let reads = seen[from..to]
                    .iter()
                    .filter(|p| p.starts_with(&root))
                    .cloned()
                    .collect();
                return Ok(Traced { text, reads });
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "the unified log did not show the trace within {}s; `log show` said: {}",
                    LOG_WAIT.as_secs(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// A profile that allows everything and reports the reads `filter` selects.
fn reporting(filter: &str) -> String {
    format!("(version 1)(allow default)(allow file-read-data {filter} (with report))")
}

/// A read of a fresh temporary file, reported to the log. One before the
/// command and one after bracket its reports: the log shows events in
/// order, so every report between them is the command's, and once the
/// second shows, every report before it has.
struct Sentinel {
    _file: tempfile::NamedTempFile,
    path: PathBuf,
}

impl Sentinel {
    fn read() -> Result<Sentinel, String> {
        let file = tempfile::NamedTempFile::new().map_err(|e| format!("trace sentinel: {e}"))?;
        let path = file
            .path()
            .canonicalize()
            .map_err(|e| format!("trace sentinel: {e}"))?;
        let profile = reporting(&format!("(literal {})", sbpl_string(&path)));
        Command::new(SANDBOX_EXEC)
            .args(["-p", &profile, "/bin/cat"])
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("{SANDBOX_EXEC}: {e}"))?;
        Ok(Sentinel { _file: file, path })
    }
}

/// The paths sandbox reports in `log show --style compact` output show
/// read, in log order: `... Sandbox: cat(325) allow file-read-data /path`.
fn sandbox_reads(log: &str) -> Vec<PathBuf> {
    const REPORT: &str = " allow file-read-data ";
    log.lines()
        .filter_map(|l| l.find(REPORT).map(|i| &l[i + REPORT.len()..]))
        .filter(|p| p.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

/// A string literal in a sandbox profile.
fn sbpl_string(p: &Path) -> String {
    let s = p.to_string_lossy();
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A string literal in a `log` predicate.
fn predicate_string(p: &Path) -> String {
    sbpl_string(p)
}

/// What one invocation of `trace` asks for.
pub struct Job<'a> {
    pub trust: bool,
    pub only: &'a [String],
    pub write: bool,
    pub verbose: bool,
    pub json: bool,
}

/// One file's part of the report.
struct Examined {
    path: PathBuf,
    error: Option<(Option<usize>, String)>,
    findings: Vec<Finding>,
}

impl Examined {
    fn new(path: &Path) -> Examined {
        Examined {
            path: path.to_path_buf(),
            error: None,
            findings: Vec::new(),
        }
    }

    fn error(path: &Path, line: Option<usize>, message: String) -> Examined {
        Examined {
            error: Some((line, message)),
            ..Examined::new(path)
        }
    }
}

/// `computed trace`: every exec region of every file, traced. Exit 2 when
/// this platform has no tracer.
pub fn main(paths: &[PathBuf], job: &Job<'_>) -> Result<u8> {
    let files = cli::discover(paths)?;
    let mut tracer = match detect() {
        Ok(t) => t,
        Err(message) => {
            let report = [Examined::error(Path::new("."), None, message)];
            finish(&report, 2, job);
            return Ok(2);
        }
    };
    let store = Store::at(Store::default_path()?);
    let mut tier = 0;
    let mut names = Vec::new();
    let mut report = Vec::new();
    for path in &files {
        let examined = examine_file(path, job, &store, tracer.as_mut(), &mut names)
            .unwrap_or_else(|e| Examined::error(path, None, format!("{e:#}")));
        tier = examined
            .findings
            .iter()
            .map(Finding::tier)
            .chain([if examined.error.is_some() { 2 } else { 0 }])
            .fold(tier, u8::max);
        if !job.json {
            print_text(&examined, job.verbose);
        }
        report.push(examined);
    }
    for name in job.only.iter().filter(|n| !names.contains(*n)) {
        let message = format!("no region is named {name:?}");
        if !job.json {
            eprintln!("computed: {message}");
        }
        report.push(Examined::error(Path::new("."), None, message));
        tier = 2;
    }
    if job.json {
        print!("{}", json(&report, tier));
    }
    Ok(tier)
}

fn finish(report: &[Examined], exit: u8, job: &Job<'_>) {
    if job.json {
        print!("{}", json(report, exit));
    } else {
        for e in report {
            print_text(e, job.verbose);
        }
    }
}

fn examine_file(
    path: &Path,
    job: &Job<'_>,
    store: &Store,
    tracer: &mut dyn Tracer,
    names: &mut Vec<String>,
) -> Result<Examined> {
    let (file, text, parsed) = match cli::open(path)? {
        Opened::Skip => return Ok(Examined::new(path)),
        Opened::Error(line, message) => return Ok(Examined::error(path, line, message)),
        Opened::Template {
            file, text, parsed, ..
        } => (file, text, parsed),
    };
    let regions: Vec<&Region> = parsed
        .segments
        .iter()
        .filter_map(|s| match s {
            Segment::Region(r) => Some(r),
            Segment::Prose(_) => None,
        })
        .collect();
    names.extend(regions.iter().filter_map(|r| r.opener.name.clone()));
    let ctx = Ctx::for_template(&file);
    let trusted = job.trust || cli::is_trusted(&ctx, store)?;
    // A `use` region's inputs= is its recipe's, in computed.toml.
    let recipes: BTreeMap<(usize, Option<usize>), String> = regions
        .iter()
        .filter_map(|r| Some((r.at(), r.opener.recipe()?.to_string())))
        .collect();
    let mut examined = Examined::new(path);
    for region in regions {
        let selected = job.only.is_empty()
            || region
                .opener
                .name
                .as_ref()
                .is_some_and(|n| job.only.contains(n));
        if !selected || !matches!(region.opener.loader.as_str(), "exec" | "transcript") {
            continue;
        }
        match examine(&ctx, region, trusted, tracer) {
            Ok(f) => examined.findings.push(f),
            Err(message) => {
                examined.findings.push(Finding {
                    line: region.line,
                    column: region.column,
                    name: region.opener.name.clone(),
                    loader: region.opener.loader.clone(),
                    verdict: Verdict::Error,
                    reads: Vec::new(),
                    undeclared: Vec::new(),
                    unused: Vec::new(),
                    suggestion: None,
                    rewritten: false,
                    message: Some(message),
                });
            }
        }
    }
    if job.write {
        let wrong = |f: &Finding| {
            matches!(
                f.verdict,
                Verdict::Undeclared | Verdict::Unused | Verdict::UndeclaredUnused
            )
        };
        for f in examined.findings.iter_mut().filter(|f| wrong(f)) {
            if let Some(recipe) = recipes.get(&(f.line, f.column)) {
                f.message = Some(format!(
                    "not rewritten: inputs= comes from [recipe.{recipe}] in computed.toml; set it there"
                ));
            }
        }
        let inputs: BTreeMap<(usize, Option<usize>), String> = examined
            .findings
            .iter()
            .filter(|f| wrong(f) && !recipes.contains_key(&(f.line, f.column)))
            .filter_map(|f| Some(((f.line, f.column), f.suggestion.clone()?)))
            .collect();
        if !inputs.is_empty() {
            match fs::replace(&file, &text, &rewrite(&parsed, &inputs)) {
                Ok(true) => {
                    for f in &mut examined.findings {
                        f.rewritten = inputs.contains_key(&(f.line, f.column));
                    }
                }
                Ok(false) => {
                    examined.error =
                        Some((None, "changed on disk while tracing; not written".into()));
                }
                Err(e) => examined.error = Some((None, format!("writing: {e}"))),
            }
        }
    }
    Ok(examined)
}

fn print_text(examined: &Examined, verbose: bool) {
    let mut err = std::io::stderr().lock();
    let path = &examined.path;
    if let Some((line, message)) = &examined.error {
        let _ = err.write_all(report::error(path, *line, message).as_bytes());
    }
    let shown: Vec<&Finding> = examined
        .findings
        .iter()
        .filter(|f| verbose || f.tier() > 0 || f.verdict == Verdict::Unused)
        .collect();
    let name_width = shown
        .iter()
        .map(|f| f.name.as_deref().map_or(0, |n| n.chars().count()))
        .max()
        .unwrap_or(0);
    for f in shown {
        let name = f.name.as_deref().unwrap_or("");
        let mut line = format!(
            "{}:{} {name:name_width$} {} {}",
            path.display(),
            marker::place(f.line, f.column),
            f.loader,
            f.verdict
        );
        if f.rewritten {
            line.push_str(" rewritten; the region is stale until `computed run`");
        }
        let mut block = format!("{line}\n");
        if let Some(m) = &f.message {
            for l in m.lines() {
                writeln!(block, "    {l}").unwrap();
            }
        }
        let listed = if f.verdict == Verdict::Volatile {
            vec![("read", &f.reads)]
        } else {
            vec![("undeclared", &f.undeclared), ("unused", &f.unused)]
        };
        for (what, paths) in listed {
            for p in paths {
                writeln!(block, "    {what}: {p}").unwrap();
            }
        }
        let differs = f.verdict != Verdict::Complete || verbose;
        if let Some(s) = f.suggestion.as_deref().filter(|_| differs && !f.rewritten) {
            writeln!(block, "    suggest: inputs={}", quoted(s)).unwrap();
        }
        let _ = err.write_all(block.as_bytes());
    }
}

/// A value as the opener writes it: double-quoted when it holds
/// whitespace or `>`.
fn quoted(value: &str) -> String {
    if value.contains([' ', '\t', '>']) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// `{"exit": n, "files": [{path, error, regions: [{line, name, verdict,
/// reads, undeclared, unused, suggestion, rewritten, message}]}]}`.
fn json(report: &[Examined], exit: u8) -> String {
    let list = |items: &[String]| {
        let inner: Vec<String> = items.iter().map(|s| report::string(s)).collect();
        format!("[{}]", inner.join(","))
    };
    let mut out = format!("{{\"exit\":{exit},\"files\":[");
    let files = report
        .iter()
        .filter(|e| e.error.is_some() || !e.findings.is_empty());
    for (i, e) in files.enumerate() {
        if i > 0 {
            out.push(',');
        }
        write!(
            out,
            "{{\"path\":{}",
            report::string(&e.path.display().to_string())
        )
        .unwrap();
        out.push_str(",\"error\":");
        match &e.error {
            Some((line, message)) => write!(
                out,
                "{{\"line\":{},\"message\":{}}}",
                line.map_or("null".to_string(), |l| l.to_string()),
                report::string(message)
            )
            .unwrap(),
            None => out.push_str("null"),
        }
        out.push_str(",\"regions\":[");
        for (j, f) in e.findings.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            write!(
                out,
                "{{\"line\":{},\"column\":{},\"name\":{},\"loader\":{},\"verdict\":{},\"reads\":{},\"undeclared\":{},\"unused\":{},\"suggestion\":{},\"rewritten\":{},\"message\":{}}}",
                f.line,
                f.column.map_or("null".to_string(), |c| c.to_string()),
                report::optional(f.name.as_deref()),
                report::string(&f.loader),
                report::string(&f.verdict.to_string()),
                list(&f.reads),
                list(&f.undeclared),
                list(&f.unused),
                report::optional(f.suggestion.as_deref()),
                f.rewritten,
                report::optional(f.message.as_deref()),
            )
            .unwrap();
        }
        out.push_str("]}");
    }
    out.push_str("]}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Answers every command with the reads listed for it, relative to the
    /// repository.
    struct Fake {
        root: PathBuf,
        reads: BTreeMap<&'static str, Vec<&'static str>>,
        calls: usize,
    }

    impl Tracer for Fake {
        fn trace(&mut self, _: &Ctx, job: &Traceable, _: &str) -> Result<Traced, String> {
            self.calls += 1;
            let cmd = match job {
                Traceable::Exec(args) => args.cmd.clone(),
                Traceable::Transcript(args) => args.steps.join(" ;; "),
            };
            if cmd == "fails" {
                return Ok(Traced {
                    text: Err(LoadError::Failed {
                        stderr: "exit status 1".into(),
                    }),
                    reads: BTreeSet::new(),
                });
            }
            Ok(Traced {
                text: Ok(String::new()),
                reads: self.reads[cmd.as_str()]
                    .iter()
                    .map(|p| {
                        if p.starts_with('/') {
                            PathBuf::from(p)
                        } else {
                            self.root.join(p)
                        }
                    })
                    .collect(),
            })
        }
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path().canonicalize().unwrap();
        for d in [".git", "docs/adr", "docs/adr/old", "src", "target"] {
            fs::create_dir_all(r.join(d)).unwrap();
        }
        fs::write(r.join(".gitignore"), "target/\n").unwrap();
        for f in [
            "docs/adr/0001.md",
            "docs/adr/0002.md",
            "docs/adr/index.txt",
            "docs/adr/old/0000.md",
            "src/main.rs",
            "src/lib.rs",
            "src/README",
            "Cargo.toml",
            "target/out.json",
            ".git/HEAD",
        ] {
            fs::write(r.join(f), f).unwrap();
        }
        fs::write(r.join("DOC.md"), "").unwrap();
        (dir, r)
    }

    fn region(opener: &str) -> Region {
        let text = format!("{opener}\n<!-- /computed -->\n");
        match marker::parse(&text).unwrap().segments.remove(0) {
            Segment::Region(r) => r,
            Segment::Prose(_) => unreachable!(),
        }
    }

    fn fake(root: &Path, reads: &[(&'static str, Vec<&'static str>)]) -> Fake {
        Fake {
            root: root.to_path_buf(),
            reads: reads.iter().cloned().collect(),
            calls: 0,
        }
    }

    #[test]
    fn reads_that_match_the_declared_inputs_are_complete() {
        let (_d, r) = repo();
        let ctx = Ctx::for_template(&r.join("DOC.md"));
        let mut t = fake(
            &r,
            &[("adrs", vec!["docs/adr/0001.md", "docs/adr/0002.md"])],
        );
        let f = examine(
            &ctx,
            &region("<!-- computed exec cmd=adrs inputs=docs/adr/*.md -->"),
            true,
            &mut t,
        )
        .unwrap();
        assert_eq!(f.verdict, Verdict::Complete);
        assert_eq!(f.tier(), 0);
        assert_eq!(f.suggestion.as_deref(), Some("docs/adr/*.md"));
    }

    #[test]
    fn a_projected_input_that_was_read_stays_projected_in_the_suggestion() {
        let (_d, r) = repo();
        let ctx = Ctx::for_template(&r.join("DOC.md"));
        let mut t = fake(&r, &[("x", vec!["Cargo.toml", "src/main.rs"])]);
        let f = examine(
            &ctx,
            &region("<!-- computed exec cmd=x inputs=Cargo.toml#lines=1 -->"),
            true,
            &mut t,
        )
        .unwrap();
        assert_eq!(f.verdict, Verdict::Undeclared);
        assert_eq!(f.undeclared, ["src/main.rs"]);
        assert_eq!(
            f.suggestion.as_deref(),
            Some("Cargo.toml#lines=1-1,src/main.rs")
        );
    }

    #[test]
    fn undeclared_reads_and_unused_inputs_are_named_with_a_suggestion() {
        let (_d, r) = repo();
        let ctx = Ctx::for_template(&r.join("DOC.md"));
        let mut t = fake(
            &r,
            &[(
                "x",
                vec![
                    "docs/adr/0001.md",
                    "Cargo.toml",
                    "/etc/hosts",
                    ".git/HEAD",
                    "DOC.md",
                    "gone.txt",
                    "src",
                ],
            )],
        );
        let f = examine(
            &ctx,
            &region("<!-- computed exec cmd=x inputs=docs/adr/*.md -->"),
            true,
            &mut t,
        )
        .unwrap();
        assert_eq!(f.verdict, Verdict::UndeclaredUnused);
        assert_eq!(f.undeclared, ["Cargo.toml"]);
        assert_eq!(f.unused, ["docs/adr/0002.md"]);
        assert_eq!(f.reads, ["Cargo.toml", "docs/adr/0001.md"]);
        assert_eq!(f.suggestion.as_deref(), Some("Cargo.toml,docs/adr/0001.md"));
        assert_eq!(f.tier(), 1);
    }

    #[test]
    fn paths_are_spelled_from_the_region_root() {
        let (_d, r) = repo();
        let ctx = Ctx::for_template(&r.join("docs/guide.md"));
        fs::write(r.join("docs/guide.md"), "").unwrap();
        let mut t = fake(&r, &[("x", vec!["Cargo.toml", "docs/adr/0001.md"])]);
        let f = examine(
            &ctx,
            &region("<!-- computed exec cmd=x inputs=adr/0001.md -->"),
            true,
            &mut t,
        )
        .unwrap();
        assert_eq!(f.verdict, Verdict::Undeclared);
        assert_eq!(f.undeclared, ["../Cargo.toml"]);
        assert_eq!(f.suggestion.as_deref(), Some("../Cargo.toml,adr/0001.md"));
    }

    #[test]
    fn a_volatile_region_shows_what_it_read_without_judging_it() {
        let (_d, r) = repo();
        let ctx = Ctx::for_template(&r.join("DOC.md"));
        let mut t = fake(&r, &[("x", vec!["Cargo.toml"])]);
        let f = examine(
            &ctx,
            &region("<!-- computed exec cmd=x volatile -->"),
            true,
            &mut t,
        )
        .unwrap();
        assert_eq!(f.verdict, Verdict::Volatile);
        assert_eq!(f.reads, ["Cargo.toml"]);
        assert_eq!(f.tier(), 0);
    }

    #[test]
    fn untrusted_regions_are_not_traced_and_failures_are_reported() {
        let (_d, r) = repo();
        let ctx = Ctx::for_template(&r.join("DOC.md"));
        let mut t = fake(&r, &[]);
        let f = examine(
            &ctx,
            &region("<!-- computed exec cmd=x volatile -->"),
            false,
            &mut t,
        )
        .unwrap();
        assert_eq!((f.verdict, t.calls), (Verdict::Untrusted, 0));
        let f = examine(
            &ctx,
            &region("<!-- computed exec cmd=fails volatile -->"),
            true,
            &mut t,
        )
        .unwrap();
        assert_eq!(f.verdict, Verdict::Failed);
        assert_eq!(f.message.as_deref(), Some("exit status 1"));
        let f = examine(
            &ctx,
            &region("<!-- computed exec cmd=x inputs=nothing/*.md -->"),
            true,
            &mut t,
        )
        .unwrap();
        assert_eq!(f.verdict, Verdict::Error);
        assert_eq!(f.tier(), 2);
    }

    fn suggestion(r: &Path, reads: &[&str]) -> Option<String> {
        let ctx = Ctx::for_template(&r.join("DOC.md"));
        suggest(&ctx, &reads.iter().map(|p| r.join(p)).collect())
    }

    #[test]
    fn a_suggestion_collapses_a_directory_only_when_the_glob_selects_exactly_the_reads() {
        let (_d, r) = repo();
        assert_eq!(
            suggestion(&r, &["docs/adr/0001.md", "docs/adr/0002.md"]).as_deref(),
            Some("docs/adr/*.md")
        );
        assert_eq!(
            suggestion(
                &r,
                &["docs/adr/0001.md", "docs/adr/0002.md", "docs/adr/index.txt"]
            )
            .as_deref(),
            Some("docs/adr/*.md,docs/adr/index.txt"),
            "docs/adr/* would bring docs/adr/old too"
        );
        assert_eq!(
            suggestion(&r, &["src/main.rs", "src/lib.rs", "src/README"]).as_deref(),
            Some("src/*")
        );
        assert_eq!(
            suggestion(&r, &["src/main.rs", "src/lib.rs"]).as_deref(),
            Some("src/*.rs")
        );
        assert_eq!(
            suggestion(&r, &["src/main.rs", "Cargo.toml"]).as_deref(),
            Some("Cargo.toml,src/main.rs")
        );
        assert_eq!(
            suggestion(&r, &["target/out.json"]).as_deref(),
            Some("target/out.json"),
            "a literal path reaches an ignored file"
        );
        assert_eq!(suggestion(&r, &[]), None);
    }

    #[test]
    fn rewrite_replaces_inputs_and_keeps_the_suffix_indent_and_line_endings() {
        let text = "a\r\n  <!-- computed exec cmd=x inputs=a name=n | do not edit; run computed -->\r\n  body\r\n  <!-- /computed in=aa out=bb -->\r\n\n<!-- computed exec   cmd=y inputs=b -->\n<!-- /computed -->\n";
        let text = text
            .replace("in=aa", &format!("in={}", "a".repeat(64)))
            .replace("out=bb", &format!("out={}", "b".repeat(64)));
        let parsed = marker::parse(&text).unwrap();
        let out = rewrite(
            &parsed,
            &BTreeMap::from([
                ((2, None), "c,d".to_string()),
                ((6, None), "e f".to_string()),
            ]),
        );
        assert!(out.contains(
            "  <!-- computed exec cmd=x inputs=c,d name=n | do not edit; run computed -->\r\n  body\r\n"
        ));
        assert!(
            out.contains("\n<!-- computed exec cmd=y inputs=\"e f\" -->\n<!-- /computed -->\n")
        );
        assert!(out.starts_with("a\r\n"));
    }

    #[test]
    fn strace_logs_give_files_opened_for_reading_and_executed() {
        let log = r#"execve("/bin/sh", ["/bin/sh", "-c", "cat a"], 0x7ffc /* 20 vars */) = 0
openat(AT_FDCWD</repo>, "/etc/ld.so.cache", O_RDONLY|O_CLOEXEC) = 3</etc/ld.so.cache>
openat(AT_FDCWD</repo>, "a", O_RDONLY) = 3</repo/a>
openat(AT_FDCWD</repo>, "missing", O_RDONLY) = -1 ENOENT (No such file or directory)
openat(AT_FDCWD</repo>, "out", O_WRONLY|O_CREAT|O_TRUNC, 0666) = 4</repo/out>
openat2(AT_FDCWD</repo>, "b", {flags=O_RDONLY|O_CLOEXEC, resolve=0}, 24) = 5</repo/sp\x20ace>
open("/abs/c", O_RDWR) = 6
execve("./run.sh", ["./run.sh"], 0x5 /* 3 vars */) = 0
execve("/nope", ["/nope"], 0x5 /* 3 vars */) = -1 ENOENT (No such file or directory)
"#;
        let reads = strace_reads(log, Path::new("/repo"));
        let reads: Vec<&str> = reads.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(
            reads,
            [
                "/bin/sh",
                "/etc/ld.so.cache",
                "/repo/a",
                "/repo/sp ace",
                "/abs/c",
                "/repo/./run.sh"
            ]
        );
    }

    #[test]
    fn sandbox_reports_give_files_read() {
        let log = "Timestamp               Ty Process[PID:TID]\n2026-09-24 18:39:12.846 Df kernel[0:bf5e83] [com.apple.sandbox.reporting:violation] Sandbox: cat(325) allow file-read-data /repo/a b.txt\n2026-09-24 18:39:12.846 Df kernel[0:bf5e83] [com.apple.sandbox.reporting:violation] 1 duplicate report for Sandbox: cat(325) allow file-read-data /repo/c\n2026-09-24 18:39:12.846 Df kernel[0:bf5e83] [com.apple.sandbox.reporting:violation] Sandbox: cat(325) allow file-read-metadata /repo/d\n";
        let reads = sandbox_reads(log);
        assert_eq!(
            reads,
            [PathBuf::from("/repo/a b.txt"), PathBuf::from("/repo/c")]
        );
    }

    #[test]
    fn relative_climbs_out_with_dot_dot() {
        assert_eq!(
            relative(Path::new("/r/docs"), Path::new("/r/src/a.rs")),
            PathBuf::from("../src/a.rs")
        );
        assert_eq!(
            relative(Path::new("/r"), Path::new("/r/a")),
            PathBuf::from("a")
        );
    }
}
