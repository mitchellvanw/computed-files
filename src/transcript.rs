//! The `transcript` loader: a shell session as a terminal would show it,
//! each step's command line after `$ ` and what it printed beneath. The
//! steps run in one `/bin/sh` in order, so a `cd` or a variable carries to
//! the next, under exec's pinned environment, process group and timeout,
//! and only in a trusted repository. A step that exits non-zero is part of
//! the transcript, not a failure of the loader.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::TempDir;

use crate::fs::{WalkOpts, walk};
use crate::launch::Wrap;
use crate::loader::{self, Ctx, LoadError, Place};
use crate::marker::Opener;
use crate::sandbox::Sandbox;

/// What separates the steps in `steps=`.
const SEPARATOR: &str = " ;; ";

/// Where the steps run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workdir {
    /// The region root, as exec runs.
    Region,
    /// A fresh empty directory, removed afterwards.
    Tmp,
    /// A copy of the repository's files that are not ignored, removed
    /// afterwards, entered at the region root's place in it.
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptArgs {
    pub steps: Vec<String>,
    /// Comma-separated globs, or `None` when volatile.
    pub inputs: Option<Vec<String>>,
    pub timeout: Duration,
    pub workdir: Workdir,
    /// Run in exec's sandbox: read only the inputs, write only the
    /// temporary directories, no network.
    pub sandbox: bool,
}

impl TranscriptArgs {
    pub fn from_opener(opener: &Opener) -> Result<TranscriptArgs, LoadError> {
        validate(opener).map_err(LoadError::Hard)?;
        Ok(TranscriptArgs {
            steps: steps(opener.attr("steps").expect("validated")),
            inputs: opener
                .attr("inputs")
                .map(|i| i.split(',').map(str::to_string).collect()),
            timeout: Duration::from_secs(
                opener
                    .attr("timeout")
                    .map_or(30, |t| t.parse().expect("validated")),
            ),
            workdir: match opener.attr("workdir") {
                None => Workdir::Region,
                Some("tmp") => Workdir::Tmp,
                Some(_) => Workdir::Copy,
            },
            sandbox: opener.flag("sandbox"),
        })
    }
}

fn steps(steps: &str) -> Vec<String> {
    steps
        .split(SEPARATOR)
        .map(|s| s.trim().to_string())
        .collect()
}

/// The opener rules the grammar checks: `steps=` required with no empty
/// step, exactly one of `inputs=` and `volatile` as for exec, `timeout=`
/// whole seconds of at least 1, `workdir=` `tmp` or `copy`.
pub fn validate(opener: &Opener) -> Result<(), String> {
    let Some(list) = opener.attr("steps") else {
        return Err("transcript needs steps=".to_string());
    };
    if let Some(i) = steps(list).iter().position(String::is_empty) {
        return Err(format!(
            "steps=: step {} is empty; steps are separated by {SEPARATOR:?}",
            i + 1
        ));
    }
    match (opener.attr("inputs"), opener.flag("volatile")) {
        (Some(_), true) => return Err("transcript takes inputs= or volatile, not both".to_string()),
        (None, false) => return Err("transcript needs inputs= or the volatile flag".to_string()),
        (Some(inputs), false)
            if inputs
                .split(',')
                .any(|g| g.trim().trim_end_matches('/').is_empty()) =>
        {
            return Err(format!(
                "inputs={inputs}: an entry is empty; remove the stray comma"
            ));
        }
        _ => {}
    }
    if let Some(inputs) = opener.attr("inputs") {
        for entry in inputs.split(',') {
            crate::project::split_input(entry.trim()).map_err(|e| format!("inputs={e}"))?;
        }
    }
    if opener.flag("sandbox") {
        if opener.flag("volatile") {
            return Err(
                "sandbox needs inputs=: the sandbox allows reading only the declared inputs"
                    .to_string(),
            );
        }
        if opener.attr("workdir") == Some("copy") {
            return Err(
                "sandbox and workdir=copy: the copy holds every file of the repository, so the sandbox could not keep reads to inputs=; use workdir=tmp, or leave the sandbox out"
                    .to_string(),
            );
        }
    }
    if let Some(t) = opener.attr("timeout")
        && t.parse::<u64>().map_or(true, |t| t == 0)
    {
        return Err(format!(
            "timeout={t}: expected seconds as a whole number of at least 1"
        ));
    }
    match opener.attr("workdir") {
        None | Some("tmp" | "copy") => Ok(()),
        Some(w) => Err(format!("workdir={w}: expected tmp or copy")),
    }
}

/// Runs the steps and returns the transcript. Each step's stdout is shown
/// before its stderr, each captured on its own, so the text does not
/// depend on how the two interleaved. The loader fails when the shell ends
/// before the last step has, on a timeout, or on output that is not UTF-8.
///
/// `wrap` goes around the shell as for exec (doctor, trace). With
/// `sandbox`, the steps run in exec's sandbox, which also lets them write
/// the capture files and a `workdir=tmp`.
pub fn run(
    ctx: &Ctx,
    args: &TranscriptArgs,
    region_name: &str,
    wrap: Option<&Wrap>,
) -> Result<String, LoadError> {
    let hard = |what: &str, e: std::io::Error| LoadError::Hard(format!("{what}: {e}"));
    let capture = tempfile::tempdir().map_err(|e| hard("temporary directory", e))?;
    let (place, workdir) = match args.workdir {
        Workdir::Region => (Place::of(ctx), None),
        Workdir::Tmp => {
            let dir = tempfile::tempdir().map_err(|e| hard("temporary directory", e))?;
            let place = Place {
                dir: canonical(dir.path())?,
                ..Place::of(ctx)
            };
            (place, Some(dir))
        }
        Workdir::Copy => {
            let (place, dir) = copy(ctx)?;
            (place, Some(dir))
        }
    };
    let ceiling = match &workdir {
        Some(dir) => canonical(dir.path())?.parent().map(Path::to_path_buf),
        None => None,
    };
    let script = script(&args.steps, &canonical(capture.path())?, ceiling.as_deref());
    let sandbox = match (&args.inputs, args.sandbox) {
        (Some(globs), true) => {
            let mut dirs = vec![capture.path()];
            dirs.extend(workdir.as_ref().map(TempDir::path));
            Some(Sandbox::new(ctx, &loader::input_files(ctx, globs)?)?.writing(&dirs)?)
        }
        _ => None,
    };
    let shell = loader::shell(
        &script,
        &place,
        args.timeout,
        region_name,
        wrap,
        sandbox.as_ref(),
    )?;
    let mut text = String::new();
    for (i, step) in args.steps.iter().enumerate() {
        let n = i + 1;
        let file = |ext: &str| capture.path().join(format!("{n}.{ext}"));
        if !file("status").exists() {
            return Err(shell.failed(format!(
                "step {n} did not finish: the shell ended with {} before it did",
                match shell.status.code() {
                    Some(c) => format!("exit status {c}"),
                    None => "a signal".to_string(),
                }
            )));
        }
        text.push_str("$ ");
        text.push_str(step);
        text.push('\n');
        for ext in ["out", "err"] {
            let bytes = std::fs::read(file(ext)).map_err(|e| hard("transcript output", e))?;
            let part = String::from_utf8(bytes)
                .map_err(|_| shell.failed(format!("step {n}: std{ext} is not UTF-8")))?;
            text.push_str(&part);
            if !part.is_empty() && !part.ends_with('\n') {
                text.push('\n');
            }
        }
    }
    Ok(text)
}

fn canonical(path: &Path) -> Result<PathBuf, LoadError> {
    path.canonicalize()
        .map_err(|e| LoadError::Hard(format!("{}: {e}", path.display())))
}

/// The one script the shell runs: each step in a `{ }` group of the shell
/// itself, its stdout and stderr to files of their own, its exit status
/// recorded and restored so the next step's `$?` is the step's. In a
/// temporary directory, `ceiling` keeps git from finding a repository above
/// it.
fn script(steps: &[String], capture: &Path, ceiling: Option<&Path>) -> String {
    let at = |n: usize, ext: &str| quote(&capture.join(format!("{n}.{ext}")).to_string_lossy());
    let mut out = String::new();
    if let Some(ceiling) = ceiling {
        out.push_str(&format!(
            "GIT_CEILING_DIRECTORIES={}; export GIT_CEILING_DIRECTORIES\n",
            quote(&ceiling.to_string_lossy())
        ));
    }
    for (i, step) in steps.iter().enumerate() {
        let n = i + 1;
        out.push_str(&format!(
            "{{ {step}\n}} >{} 2>{}\n_computed_status=$?; echo \"$_computed_status\" >{}; (exit \"$_computed_status\")\n",
            at(n, "out"),
            at(n, "err"),
            at(n, "status"),
        ));
    }
    out
}

/// `s` as one single-quoted shell word.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A copy of the files the tree loader would list under the repository
/// root (the region root outside one), dotfiles included, ignored files and
/// `.git` left out, symlinks copied as symlinks. An empty `.git` marks the
/// copy as a repository root, so `computed` run inside it resolves paths
/// and ignore rules as it does in the original; `git` finds no repository
/// there. `COMPUTED_ROOT` and `COMPUTED_FILE` name the copy's.
fn copy(ctx: &Ctx) -> Result<(Place, TempDir), LoadError> {
    let hard = |what: &Path, e: std::io::Error| {
        LoadError::Hard(format!("workdir=copy: {}: {e}", what.display()))
    };
    let root = match &ctx.repo_root {
        Some(r) => r.clone(),
        None => canonical(&ctx.region_root)?,
    };
    let temp = tempfile::tempdir().map_err(|e| hard(Path::new("temporary directory"), e))?;
    let dest = canonical(temp.path())?;
    let opts = WalkOpts {
        all: true,
        ..WalkOpts::default()
    };
    for entry in walk(&root, opts) {
        let from = root.join(&entry.path);
        let to = dest.join(&entry.path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| hard(parent, e))?;
        }
        if entry.is_link {
            let target = std::fs::read_link(&from).map_err(|e| hard(&from, e))?;
            std::os::unix::fs::symlink(target, &to).map_err(|e| hard(&to, e))?;
        } else if entry.is_dir {
            std::fs::create_dir_all(&to).map_err(|e| hard(&to, e))?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| hard(&from, e))?;
        }
    }
    if ctx.repo_root.is_some() {
        std::fs::create_dir_all(dest.join(".git")).map_err(|e| hard(&dest, e))?;
    }
    let inside = |path: &Path| -> Result<PathBuf, LoadError> {
        let path = canonical(path)?;
        let rel = path.strip_prefix(&root).map_err(|_| {
            LoadError::Hard(format!(
                "workdir=copy: {} is outside {}",
                path.display(),
                root.display()
            ))
        })?;
        Ok(dest.join(rel))
    };
    let place = Place {
        dir: inside(&ctx.region_root)?,
        file: inside(&ctx.template)?,
        root: ctx.repo_root.as_ref().map(|_| dest.clone()),
    };
    Ok((place, temp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::{self, Segment};
    use std::fs;

    fn args(attrs: &str) -> Result<TranscriptArgs, String> {
        let text = format!("<!-- computed transcript {attrs} -->\n<!-- /computed -->\n");
        let file = marker::parse(&text).map_err(|e| e.message)?;
        match &file.segments[0] {
            Segment::Region(r) => {
                TranscriptArgs::from_opener(&r.opener).map_err(|e| format!("{e:?}"))
            }
            Segment::Prose(_) => unreachable!(),
        }
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::create_dir_all(r.join("docs")).unwrap();
        fs::create_dir_all(r.join("target")).unwrap();
        fs::write(r.join(".gitignore"), "target/\n").unwrap();
        fs::write(r.join("a.txt"), "a\n").unwrap();
        fs::write(r.join("target/big"), "").unwrap();
        fs::write(r.join("docs/guide.md"), "").unwrap();
        dir
    }

    fn transcript(root: &Path, template: &str, attrs: &str) -> Result<String, LoadError> {
        let ctx = Ctx::for_template(&root.join(template));
        run(&ctx, &args(attrs).unwrap(), "transcript@1", None)
    }

    #[test]
    fn the_opener_names_steps_and_inputs_or_volatile() {
        let a = args("steps=\"echo a ;; echo b\" volatile workdir=copy timeout=5").unwrap();
        assert_eq!(a.steps, ["echo a", "echo b"]);
        assert_eq!(a.inputs, None);
        assert_eq!(a.workdir, Workdir::Copy);
        assert_eq!(a.timeout, Duration::from_secs(5));
        for (attrs, message) in [
            ("volatile", "transcript needs steps="),
            ("steps=x", "transcript needs inputs= or the volatile flag"),
            (
                "steps=x inputs=a volatile",
                "transcript takes inputs= or volatile, not both",
            ),
            (
                "steps=\"a ;;  ;; b\" volatile",
                "steps=: step 2 is empty; steps are separated by \" ;; \"",
            ),
            (
                "steps=x inputs=a,",
                "inputs=a,: an entry is empty; remove the stray comma",
            ),
            (
                "steps=x volatile timeout=0",
                "timeout=0: expected seconds as a whole number of at least 1",
            ),
            (
                "steps=x volatile workdir=home",
                "workdir=home: expected tmp or copy",
            ),
        ] {
            assert_eq!(args(attrs).unwrap_err(), message, "{attrs}");
        }
    }

    #[test]
    fn steps_share_one_shell_and_show_stdout_then_stderr() {
        let dir = repo();
        let text = transcript(
            dir.path(),
            "README.md",
            "steps=\"echo err >&2; echo out ;; X=kept; cd docs ;; echo $X; ls ;; false ;; echo $? ;; printf partial\" volatile",
        )
        .unwrap();
        assert_eq!(
            text,
            "$ echo err >&2; echo out\nout\nerr\n$ X=kept; cd docs\n$ echo $X; ls\nkept\nguide.md\n$ false\n$ echo $?\n1\n$ printf partial\npartial\n"
        );
    }

    #[test]
    fn a_shell_that_ends_early_or_runs_out_of_time_fails_the_loader() {
        let dir = repo();
        let e = transcript(
            dir.path(),
            "README.md",
            "steps=\"echo a ;; exit 3 ;; echo c\" volatile",
        )
        .unwrap_err();
        assert!(
            matches!(&e, LoadError::Failed { stderr } if stderr == "step 2 did not finish: the shell ended with exit status 3 before it did"),
            "{e:?}"
        );
        let e =
            transcript(dir.path(), "README.md", "steps=\"echo 'unclosed\" volatile").unwrap_err();
        assert!(
            matches!(&e, LoadError::Failed { stderr } if stderr.starts_with("step 1 did not finish")),
            "{e:?}"
        );
        let e = transcript(
            dir.path(),
            "README.md",
            "steps=\"sleep 5\" volatile timeout=1",
        )
        .unwrap_err();
        assert!(
            matches!(&e, LoadError::Failed { stderr } if stderr.contains("timed out")),
            "{e:?}"
        );
    }

    #[test]
    fn workdir_tmp_is_empty_and_workdir_copy_leaves_the_repository_alone() {
        let dir = repo();
        let r = dir.path();
        let root = r.canonicalize().unwrap();
        let text = transcript(
            r,
            "README.md",
            "steps=\"ls -A ;; touch new.txt ;; echo $COMPUTED_ROOT\" volatile workdir=tmp",
        )
        .unwrap();
        assert_eq!(
            text,
            format!(
                "$ ls -A\n$ touch new.txt\n$ echo $COMPUTED_ROOT\n{}\n",
                root.display()
            )
        );
        assert!(!r.join("new.txt").exists());

        let text = transcript(
            r,
            "docs/guide.md",
            "steps=\"pwd -P ;; ls -A .. ;; rm ../a.txt ;; ls .. ;; echo $COMPUTED_FILE\" volatile workdir=copy",
        )
        .unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let copy = Path::new(lines[1]);
        assert_eq!(copy.file_name().unwrap(), "docs");
        assert_ne!(copy, root.join("docs"));
        assert_eq!(
            &lines[2..],
            [
                "$ ls -A ..",
                ".git",
                ".gitignore",
                "a.txt",
                "docs",
                "$ rm ../a.txt",
                "$ ls ..",
                "docs",
                "$ echo $COMPUTED_FILE",
                &copy.join("guide.md").to_string_lossy(),
            ]
        );
        assert!(r.join("a.txt").exists(), "the original is untouched");
        assert!(!copy.exists(), "the copy is removed");
    }

    #[test]
    fn the_script_quotes_its_paths() {
        assert_eq!(quote("a'b c"), "'a'\\''b c'");
        assert_eq!(
            script(&["echo hi".to_string()], Path::new("/t"), None),
            "{ echo hi\n} >'/t/1.out' 2>'/t/1.err'\n_computed_status=$?; echo \"$_computed_status\" >'/t/1.status'; (exit \"$_computed_status\")\n"
        );
    }
}
