//! `computed doctor`: runs every region's loader twice, once as `run` would
//! and once from another directory under a perturbed environment, and says
//! whether the two agree. A fresh region whose loader now prints something
//! other than its body has moved: its output changed without its inputs,
//! which `check` cannot see ([ADR 0006]). Writes nothing.
//!
//! [ADR 0006]: ../docs/adr/0006-check-never-runs-a-loader.md

use std::ffi::OsString;
use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::allow::Allowed;
use crate::cli::{self, Opened};
use crate::launch::{self, Wrap};
use crate::loader::{Ctx, Production};
use crate::marker::{self, File, Region, Segment};
use crate::render::{self, Action, Mode, Rendered, State};
use crate::report;
use crate::trust::Store;

/// What the two runs of a region's loader came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Both runs wrote the same body.
    Deterministic,
    /// The runs differ, or only the perturbed one failed.
    Nondeterministic,
    /// The loader failed as `run` runs it.
    Failed,
    /// An exec region in a repository without a grant.
    Untrusted,
    /// A remote region whose url the allowlist does not cover.
    Disallowed,
    /// The tool could not answer for the region.
    Error,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Verdict::Deterministic => "deterministic",
            Verdict::Nondeterministic => "nondeterministic",
            Verdict::Failed => "failed",
            Verdict::Untrusted => "untrusted",
            Verdict::Disallowed => "disallowed",
            Verdict::Error => "error",
        })
    }
}

/// One region's examination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    /// The opener's column for a region inside a line.
    pub column: Option<usize>,
    pub name: Option<String>,
    pub loader: String,
    pub verdict: Verdict,
    /// The diff from the file's body to what the loader prints now, when a
    /// fresh region's output moved without its inputs.
    pub moved: Option<String>,
    /// The diff from the first run's body to the perturbed run's.
    pub diff: Option<String>,
    /// Loader stderr, or why the tool could not answer.
    pub message: Option<String>,
}

impl Finding {
    /// The exit tier this region contributes.
    pub fn tier(&self) -> u8 {
        match self.verdict {
            Verdict::Error => 2,
            Verdict::Deterministic if self.moved.is_none() => 0,
            _ => 1,
        }
    }
}

/// The region bodies a render produced, in file order.
fn bodies(parsed: &File, rendered: &Rendered) -> Result<Vec<String>, (usize, String)> {
    let file = match rendered {
        Rendered::Written { text, .. } => {
            marker::parse_as(text, parsed.syntax).map_err(|e| (e.line, e.message))?
        }
        Rendered::Unchanged { .. } | Rendered::Refused { .. } => parsed.clone(),
        Rendered::Error { line, message } => return Err((*line, message.clone())),
    };
    Ok(regions(&file).map(|r| r.body.clone()).collect())
}

fn regions(file: &File) -> impl Iterator<Item = &Region> {
    file.segments.iter().filter_map(|s| match s {
        Segment::Region(r) => Some(r),
        Segment::Prose(_) => None,
    })
}

/// Compares two forced renders of `parsed`, the one `run` would make and
/// the perturbed one, region by region. Pure. An error is the file's.
pub fn compare(
    parsed: &File,
    first: &Rendered,
    second: &Rendered,
) -> Result<Vec<Finding>, (usize, String)> {
    let one = bodies(parsed, first)?;
    let two = bodies(parsed, second)?;
    let mut findings = Vec::new();
    for (i, region) in regions(parsed).enumerate() {
        let find = |rendered: &Rendered| {
            rendered
                .regions()
                .iter()
                .find(|r| (r.line, r.column) == region.at())
                .cloned()
        };
        // A region `select` left out has no report.
        let (Some(a), Some(b)) = (find(first), find(second)) else {
            continue;
        };
        let mut finding = Finding {
            line: region.line,
            column: region.column,
            name: region.opener.name.clone(),
            loader: region.opener.loader.clone(),
            verdict: Verdict::Deterministic,
            moved: None,
            diff: None,
            message: None,
        };
        let perturbed =
            |m: Option<String>| Some(format!("perturbed run: {}", m.unwrap_or_default()));
        match (a.action, b.action) {
            (Some(Action::Error), _) => {
                finding.verdict = Verdict::Error;
                finding.message = a.stderr;
            }
            (Some(Action::Untrusted), _) => finding.verdict = Verdict::Untrusted,
            (Some(Action::Disallowed), _) => {
                finding.verdict = Verdict::Disallowed;
                finding.message = a.stderr;
            }
            (Some(Action::Failed), _) => {
                finding.verdict = Verdict::Failed;
                finding.message = a.stderr;
            }
            (_, Some(Action::Error)) => {
                finding.verdict = Verdict::Error;
                finding.message = perturbed(b.stderr);
            }
            (_, Some(Action::Failed)) => {
                finding.verdict = Verdict::Nondeterministic;
                finding.message = perturbed(b.stderr);
            }
            _ => {
                if one[i] != two[i] {
                    finding.verdict = Verdict::Nondeterministic;
                    finding.diff = Some(short_diff(&one[i], &two[i], "run", "perturbed run"));
                }
                if a.state == State::Fresh && one[i] != region.body {
                    finding.moved = Some(short_diff(&region.body, &one[i], "file", "loader now"));
                }
            }
        }
        findings.push(finding);
    }
    Ok(findings)
}

/// The most lines of a diff a report shows.
const DIFF_LINES: usize = 12;

fn short_diff(old: &str, new: &str, old_name: &str, new_name: &str) -> String {
    let diff = similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(1)
        .header(old_name, new_name)
        .to_string();
    let lines: Vec<&str> = diff.lines().collect();
    let mut out = lines[..lines.len().min(DIFF_LINES)].join("\n");
    if lines.len() > DIFF_LINES {
        write!(out, "\n... {} more lines", lines.len() - DIFF_LINES).unwrap();
    }
    out
}

/// A scratch directory under the system temp dir, removed on drop, that
/// the perturbed run takes its home, temp dir and working directory from.
pub struct Perturbation {
    dir: tempfile::TempDir,
    noise: String,
}

impl Perturbation {
    pub fn new() -> std::io::Result<Perturbation> {
        let dir = tempfile::Builder::new()
            .prefix("computed-doctor")
            .tempdir()?;
        for sub in ["home", "tmp", "cwd"] {
            std::fs::create_dir(dir.path().join(sub))?;
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let noise = format!("{:08x}", nanos ^ std::process::id());
        Ok(Perturbation { dir, noise })
    }

    /// The directory the invocation moves to for the perturbed run.
    pub fn cwd(&self) -> PathBuf {
        self.dir.path().join("cwd")
    }

    /// Restarts the command under `env -i` with another `HOME`, `TMPDIR`,
    /// `USER`, `LOGNAME`, `PWD` and `OLDPWD`, one variable more, and the
    /// variables in reverse order.
    pub fn wrap(&self) -> Box<Wrap> {
        let root = self.dir.path().to_path_buf();
        let noise = self.noise.clone();
        Box::new(move |command: Command| {
            let mut env = launch::environment(&command);
            let mut set = |k: &str, v: OsString| env.insert(k.into(), v);
            set("HOME", root.join("home").into());
            set("TMPDIR", root.join("tmp").into());
            set("USER", "computed-doctor".into());
            set("LOGNAME", "computed-doctor".into());
            set("PWD", root.join("cwd").into());
            set("OLDPWD", root.join("cwd").into());
            set(&format!("COMPUTED_DOCTOR_{noise}"), noise.clone().into());
            let mut prefix: Vec<OsString> = vec!["/usr/bin/env".into(), "-i".into()];
            // `env` would read a name starting with `-` as an option.
            for (k, v) in env
                .iter()
                .rev()
                .filter(|(k, _)| !k.to_string_lossy().starts_with('-'))
            {
                let mut kv = k.clone();
                kv.push("=");
                kv.push(v);
                prefix.push(kv);
            }
            Ok(launch::prefixed(&command, prefix))
        })
    }
}

/// Runs `f` with the invocation in another working directory and under
/// another umask, and puts both back.
fn elsewhere<T>(cwd: &Path, f: impl FnOnce() -> T) -> Result<T> {
    let back = std::env::current_dir().context("current directory")?;
    // SAFETY: umask(2) only swaps the process's file mode mask.
    let mask = unsafe { libc::umask(0o077) };
    if mask == 0o077 {
        // SAFETY: as above.
        unsafe { libc::umask(0o002) };
    }
    std::env::set_current_dir(cwd).with_context(|| format!("{}", cwd.display()))?;
    let out = f();
    let restored = std::env::set_current_dir(&back);
    // SAFETY: as above.
    unsafe { libc::umask(mask) };
    restored.with_context(|| format!("{}", back.display()))?;
    Ok(out)
}

/// What one invocation of `doctor` asks for.
pub struct Job<'a> {
    pub trust: bool,
    pub only: &'a [String],
    /// The url prefixes `remote` regions may fetch under, as for `run`.
    pub allowed: &'a Allowed,
    pub verbose: bool,
    pub json: bool,
}

/// One file's part of the report.
struct Examined {
    path: PathBuf,
    error: Option<(Option<usize>, String)>,
    findings: Vec<Finding>,
}

/// `computed doctor`: every file, every selected region, twice.
pub fn main(paths: &[PathBuf], job: &Job<'_>) -> Result<u8> {
    let files = cli::discover(paths)?;
    let store = Store::at(Store::default_path()?);
    let perturbation = Perturbation::new().context("scratch directory")?;
    let mut tier = 0;
    let mut names = Vec::new();
    let mut report = Vec::new();
    for path in &files {
        let examined = match examine(path, job, &store, &perturbation, &mut names) {
            Ok(e) => e,
            Err(e) => Examined {
                path: path.clone(),
                error: Some((None, format!("{e:#}"))),
                findings: Vec::new(),
            },
        };
        if examined.error.is_some() {
            tier = 2;
        }
        for f in &examined.findings {
            tier = tier.max(f.tier());
        }
        if !job.json {
            print_text(&examined, job.verbose);
        }
        report.push(examined);
    }
    for name in job.only.iter().filter(|n| !names.contains(*n)) {
        let message = format!("no region is named {name:?}");
        if job.json {
            report.push(Examined {
                path: PathBuf::from("."),
                error: Some((None, message)),
                findings: Vec::new(),
            });
        } else {
            eprintln!("computed: {message}");
        }
        tier = 2;
    }
    if job.json {
        print!("{}", json(&report, tier));
    }
    Ok(tier)
}

fn examine(
    path: &Path,
    job: &Job<'_>,
    store: &Store,
    perturbation: &Perturbation,
    names: &mut Vec<String>,
) -> Result<Examined> {
    let mut examined = Examined {
        path: path.to_path_buf(),
        error: None,
        findings: Vec::new(),
    };
    let (file, parsed, recipes) = match cli::open(path)? {
        Opened::Skip => return Ok(examined),
        Opened::Error(line, message) => {
            examined.error = Some((line, message));
            return Ok(examined);
        }
        Opened::Template {
            file,
            parsed,
            recipes,
            ..
        } => (file, parsed, recipes),
    };
    names.extend(regions(&parsed).filter_map(|r| r.opener.name.clone()));
    let ctx = Ctx::for_template(&file);
    let trusted = job.trust || cli::is_trusted(&ctx, store)?;
    let select = |r: &Region| {
        job.only.is_empty() || r.opener.name.as_ref().is_some_and(|n| job.only.contains(n))
    };
    let mode = Mode::Run { force: true };
    let loaders = |ctx| {
        Production::new(ctx)
            .allowing(job.allowed.clone())
            .with_recipes(&recipes)
    };
    let first = render::file_where(&parsed, mode, trusted, &select, &mut loaders(ctx));
    // Absolute paths, so the template is found from the other directory.
    let far = Ctx::for_template(&file.canonicalize().context("unreadable")?);
    let mut loaders = loaders(far).with_wrap(perturbation.wrap());
    let second = elsewhere(&perturbation.cwd(), || {
        render::file_where(&parsed, mode, trusted, &select, &mut loaders)
    })?;
    match compare(&parsed, &first, &second) {
        Ok(findings) => examined.findings = findings,
        Err((line, message)) => examined.error = Some((Some(line), message)),
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
        .filter(|f| verbose || f.tier() > 0)
        .collect();
    let name_width = shown
        .iter()
        .map(|f| f.name.as_deref().map_or(0, |n| n.chars().count()))
        .max()
        .unwrap_or(0);
    let verdict_width = shown
        .iter()
        .map(|f| f.verdict.to_string().len())
        .max()
        .unwrap_or(0);
    for f in shown {
        let name = f.name.as_deref().unwrap_or("");
        let mut line = format!(
            "{}:{} {name:name_width$} {} {:verdict_width$}",
            path.display(),
            crate::marker::place(f.line, f.column),
            f.loader,
            f.verdict.to_string()
        );
        if f.moved.is_some() {
            line.push_str(" moved; `computed run --force` catches it up");
        }
        let mut block = format!("{}\n", line.trim_end());
        for text in [&f.message, &f.diff, &f.moved].into_iter().flatten() {
            for l in text.lines() {
                writeln!(block, "    {l}").unwrap();
            }
        }
        let _ = err.write_all(block.as_bytes());
    }
}

/// `{"exit": n, "files": [{path, error, regions: [{line, name, loader,
/// verdict, moved, diff, moved_diff, message}]}]}`, files with nothing to
/// say left out.
fn json(report: &[Examined], exit: u8) -> String {
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
                "{{\"line\":{},\"name\":{},\"loader\":{},\"verdict\":{},\"moved\":{},\"diff\":{},\"moved_diff\":{},\"message\":{}}}",
                f.line,
                report::optional(f.name.as_deref()),
                report::string(&f.loader),
                report::string(&f.verdict.to_string()),
                f.moved.is_some(),
                report::optional(f.diff.as_deref()),
                report::optional(f.moved.as_deref()),
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
    use crate::loader::{LoadError, Loaded};
    use crate::render::Loaders;

    /// Answers every region with one snapshot and a text per region name.
    struct Fake {
        text: fn(&str) -> Result<String, LoadError>,
    }

    impl Loaders for Fake {
        fn snapshot(&mut self, region: &Region) -> Result<Option<Vec<u8>>, LoadError> {
            match region.opener.name.as_deref() {
                Some("broken") => Err(LoadError::Hard("no such input".into())),
                _ => Ok(Some(b"snap".to_vec())),
            }
        }
        fn load(&mut self, region: &Region) -> Result<Loaded, LoadError> {
            Ok(Loaded {
                text: (self.text)(region.opener.name.as_deref().unwrap())?,
                snapshot: b"snap".to_vec(),
            })
        }
    }

    const TEMPLATE: &str = "<!-- computed exec cmd=a inputs=x name=a -->\n<!-- /computed -->\n\n<!-- computed exec cmd=b inputs=x name=b -->\n<!-- /computed -->\n";

    /// The template rendered once by `run` with the text `a` and `b`.
    fn rendered() -> File {
        let parsed = marker::parse(TEMPLATE).unwrap();
        let mut fake = Fake {
            text: |n| Ok(n.to_string()),
        };
        let Rendered::Written { text, .. } =
            render::file(&parsed, Mode::Run { force: false }, true, &mut fake)
        else {
            panic!("the first run writes");
        };
        marker::parse(&text).unwrap()
    }

    fn doctor(
        parsed: &File,
        trusted: bool,
        one: fn(&str) -> Result<String, LoadError>,
        two: fn(&str) -> Result<String, LoadError>,
    ) -> Vec<Finding> {
        let mode = Mode::Run { force: true };
        let first = render::file(parsed, mode, trusted, &mut Fake { text: one });
        let second = render::file(parsed, mode, trusted, &mut Fake { text: two });
        compare(parsed, &first, &second).unwrap()
    }

    fn verdicts(findings: &[Finding]) -> Vec<(String, Verdict, bool)> {
        findings
            .iter()
            .map(|f| (f.name.clone().unwrap(), f.verdict, f.moved.is_some()))
            .collect()
    }

    #[test]
    fn two_runs_that_agree_are_deterministic_and_a_fresh_body_has_not_moved() {
        let parsed = rendered();
        let same = |n: &str| Ok(n.to_string());
        let findings = doctor(&parsed, true, same, same);
        assert_eq!(
            verdicts(&findings),
            [
                ("a".into(), Verdict::Deterministic, false),
                ("b".into(), Verdict::Deterministic, false)
            ]
        );
        assert!(findings.iter().all(|f| f.tier() == 0));
    }

    #[test]
    fn runs_that_differ_are_nondeterministic_with_a_diff() {
        let parsed = rendered();
        let findings = doctor(
            &parsed,
            true,
            |n| Ok(n.to_string()),
            |n| Ok(if n == "b" { "B".into() } else { n.into() }),
        );
        assert_eq!(findings[0].verdict, Verdict::Deterministic);
        assert_eq!(findings[1].verdict, Verdict::Nondeterministic);
        let diff = findings[1].diff.as_deref().unwrap();
        assert!(diff.contains("-b") && diff.contains("+B"), "{diff}");
        assert_eq!(findings[1].tier(), 1);
    }

    #[test]
    fn a_fresh_region_whose_loader_prints_something_else_has_moved() {
        let parsed = rendered();
        let moved = |n: &str| Ok(if n == "a" { "a2".into() } else { n.into() });
        let findings = doctor(&parsed, true, moved, moved);
        assert_eq!(
            verdicts(&findings),
            [
                ("a".into(), Verdict::Deterministic, true),
                ("b".into(), Verdict::Deterministic, false)
            ]
        );
        let diff = findings[0].moved.as_deref().unwrap();
        assert!(diff.contains("-a") && diff.contains("+a2"), "{diff}");
        assert_eq!(findings[0].tier(), 1);
    }

    #[test]
    fn an_unrendered_region_is_not_moved_whatever_it_prints() {
        let parsed = marker::parse(TEMPLATE).unwrap();
        let same = |n: &str| Ok(n.to_string());
        let findings = doctor(&parsed, true, same, same);
        assert!(findings.iter().all(|f| f.moved.is_none() && f.tier() == 0));
    }

    #[test]
    fn untrusted_failed_and_perturbed_failures_are_reported() {
        let parsed = rendered();
        let same = |n: &str| Ok(n.to_string());
        let findings = doctor(&parsed, false, same, same);
        assert!(findings.iter().all(|f| f.verdict == Verdict::Untrusted));

        let fails = |_: &str| {
            Err(LoadError::Failed {
                stderr: "exit status 1".into(),
            })
        };
        let findings = doctor(&parsed, true, fails, same);
        assert_eq!(findings[0].verdict, Verdict::Failed);
        assert_eq!(findings[0].message.as_deref(), Some("exit status 1"));

        let findings = doctor(&parsed, true, same, fails);
        assert_eq!(findings[0].verdict, Verdict::Nondeterministic);
        assert_eq!(
            findings[0].message.as_deref(),
            Some("perturbed run: exit status 1")
        );
    }

    #[test]
    fn a_region_the_tool_cannot_answer_is_an_error() {
        let parsed = marker::parse(
            "<!-- computed exec cmd=a inputs=x name=broken -->\n<!-- /computed -->\n",
        )
        .unwrap();
        let same = |n: &str| Ok(n.to_string());
        let findings = doctor(&parsed, true, same, same);
        assert_eq!(findings[0].verdict, Verdict::Error);
        assert_eq!(findings[0].tier(), 2);
    }

    #[test]
    fn the_perturbation_changes_the_environment_and_reverses_it() {
        let p = Perturbation::new().unwrap();
        let mut c = Command::new("/bin/sh");
        c.arg("-c")
            .arg("env")
            .env("HOME", "/nowhere")
            .env("AAA", "1");
        let wrapped = (p.wrap())(c).unwrap();
        assert_eq!(wrapped.get_program(), "/usr/bin/env");
        let args: Vec<String> = wrapped
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-i");
        assert_eq!(&args[args.len() - 3..], ["/bin/sh", "-c", "env"]);
        let vars = &args[1..args.len() - 3];
        let home = format!("HOME={}", p.dir.path().join("home").display());
        assert!(vars.contains(&home), "{vars:?}");
        assert!(vars.iter().any(|v| v.starts_with("COMPUTED_DOCTOR_")));
        let keys: Vec<&str> = vars.iter().map(|v| v.split('=').next().unwrap()).collect();
        let mut reversed = keys.clone();
        reversed.sort();
        reversed.reverse();
        assert_eq!(keys, reversed, "the variables are in reverse order");
    }
}
