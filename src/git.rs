//! The `git` loader: recent commits touching a path, the most recent tags,
//! or the authors of a path, read from the repository with the `git`
//! binary. Reading history runs no repository code, so it needs no trust,
//! and `check` runs the same query to take the snapshot.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::loader::{Ctx, LoadError, Loaded};
use crate::marker::Opener;

/// Commits and tags shown when `n=` is not given.
const DEFAULT_COUNT: usize = 10;

/// The query, named by the flag after `git`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    /// The last `n` commits touching `src`.
    Log { src: PathBuf, n: usize },
    /// The `n` most recently created tags.
    Tags { n: usize },
    /// Every author of a commit touching `src`, in order of first commit.
    Contributors { src: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitArgs {
    pub query: Query,
}

const QUERIES: [&str; 3] = ["log", "tags", "contributors"];

impl GitArgs {
    pub fn from_opener(opener: &Opener) -> Result<GitArgs, LoadError> {
        validate(opener).map_err(LoadError::Hard)?;
        let src = || PathBuf::from(opener.attr("src").unwrap_or("."));
        let n = opener
            .attr("n")
            .map_or(DEFAULT_COUNT, |n| n.parse().expect("validated"));
        let query = if opener.flag("log") {
            Query::Log { src: src(), n }
        } else if opener.flag("tags") {
            Query::Tags { n }
        } else {
            Query::Contributors { src: src() }
        };
        Ok(GitArgs { query })
    }
}

/// The opener rules the grammar checks: exactly one query flag, `n=` a
/// whole number of at least 1 for `log` and `tags`, `src=` not on `tags`.
pub fn validate(opener: &Opener) -> Result<(), String> {
    let queries: Vec<&str> = QUERIES.into_iter().filter(|q| opener.flag(q)).collect();
    let query = match queries.as_slice() {
        [q] => *q,
        [] => return Err("git needs one of log, tags or contributors".to_string()),
        _ => {
            return Err(format!(
                "git takes one query, not {}",
                queries.join(" and ")
            ));
        }
    };
    if query == "tags" && opener.attr("src").is_some() {
        return Err("git tags takes no src=: tags belong to the repository".to_string());
    }
    match opener.attr("n") {
        Some(_) if query == "contributors" => {
            Err("git contributors lists every author; n= is for log and tags".to_string())
        }
        Some(n) if n.parse::<usize>().map_or(true, |n| n == 0) => {
            Err(format!("n={n}: expected a whole number of at least 1"))
        }
        _ => Ok(()),
    }
}

/// Runs the query. The text is a Markdown list, one item per commit, tag or
/// author; the snapshot is what the text is made from, one per line: full
/// commit ids for `log`, whose subjects they fix, and the names themselves
/// for `tags` and `contributors`.
pub fn load(ctx: &Ctx, args: &GitArgs) -> Result<Loaded, LoadError> {
    if ctx.repo_root.is_none() {
        return Err(LoadError::Hard(
            "git: the template is not inside a git repository".to_string(),
        ));
    }
    if git(ctx, &["rev-parse", "--is-shallow-repository"])?.trim() == "true" {
        return Err(LoadError::Hard(
            "git: the repository is a shallow clone, so its history is cut short; fetch all of it (`git fetch --unshallow`, or `fetch-depth: 0` in actions/checkout)".to_string(),
        ));
    }
    let (items, ids): (Vec<String>, Vec<String>) = match &args.query {
        Query::Log { src, n } => {
            let src = pathspec(ctx, src)?;
            let out = git(
                ctx,
                &[
                    "log",
                    "--no-color",
                    "--format=%H%x00%s",
                    &format!("--max-count={n}"),
                    "--",
                    &src,
                ],
            )?;
            out.lines()
                .filter_map(|l| l.split_once('\0'))
                .map(|(id, subject)| {
                    (
                        format!("{} {subject}", &id[..id.len().min(7)]),
                        id.to_string(),
                    )
                })
                .unzip()
        }
        Query::Tags { n } => {
            // The last key sorts first: newest first, ties by name.
            let out = git(
                ctx,
                &[
                    "for-each-ref",
                    "--sort=refname",
                    "--sort=-creatordate",
                    &format!("--count={n}"),
                    "--format=%(refname:strip=2)",
                    "refs/tags",
                ],
            )?;
            out.lines().map(|t| (t.to_string(), t.to_string())).unzip()
        }
        Query::Contributors { src } => {
            let src = pathspec(ctx, src)?;
            let out = git(ctx, &["log", "--reverse", "--format=%aN", "--", &src])?;
            let mut names: Vec<String> = Vec::new();
            for name in out.lines() {
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
            names.iter().map(|n| (n.clone(), n.clone())).unzip()
        }
    };
    let text: String = items.iter().map(|i| format!("- {i}\n")).collect();
    let snapshot: Vec<u8> = ids
        .iter()
        .flat_map(|i| format!("{i}\n").into_bytes())
        .collect();
    Ok(Loaded { text, snapshot })
}

/// `src=` checked like every marker path, then passed as written: git runs
/// in the region root, so the relative path means what it says.
fn pathspec(ctx: &Ctx, src: &Path) -> Result<String, LoadError> {
    ctx.resolve("src=", src)?;
    Ok(src.to_string_lossy().into_owned())
}

/// Environment that would point git somewhere other than the repository
/// the region root is in, or change what it prints: a pre-commit hook sets
/// `GIT_DIR` and `GIT_INDEX_FILE`, and `git -c` passes its settings down.
const CLEARED: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
    "GIT_GRAFT_FILE",
];

/// `git` in `dir` with what changes its output pinned: the C locale and
/// UTC, no replace refs, pathspecs taken literally, settings that reshape
/// `log` overridden on the command line, no fsmonitor, which would run a
/// configured command, and none of the environment a hook sets. The user's
/// and the system's configuration still load, so `safe.directory` keeps
/// working. Every command that asks the repository something builds on it.
pub(crate) fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .args([
            "--no-pager",
            "--no-replace-objects",
            "--literal-pathspecs",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "log.showSignature=false",
            "-c",
            "log.follow=false",
            "-c",
            "i18n.logOutputEncoding=UTF-8",
        ])
        .current_dir(dir)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env("LANGUAGE", "")
        .env("TZ", "UTC")
        .env("GIT_TERMINAL_PROMPT", "0");
    for var in CLEARED {
        command.env_remove(var);
    }
    command
}

/// Runs `git` in the region root through [`command`]. Any failure is hard:
/// the tool could not read the history.
fn git(ctx: &Ctx, args: &[&str]) -> Result<String, LoadError> {
    let mut command = command(&ctx.region_root);
    command.args(args);
    let out = command
        .output()
        .map_err(|e| LoadError::Hard(format!("git: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(LoadError::Hard(format!(
            "git {}: {}",
            args[0],
            stderr.trim_end()
        )));
    }
    String::from_utf8(out.stdout)
        .map_err(|_| LoadError::Hard(format!("git {}: output is not UTF-8", args[0])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::{self, Segment};
    use std::fs;

    /// A repository with pinned identities and dates, whatever the user's
    /// configuration says.
    struct Repo {
        dir: tempfile::TempDir,
        clock: u64,
    }

    impl Repo {
        fn new() -> Repo {
            let repo = Repo {
                dir: tempfile::tempdir().unwrap(),
                clock: 1_700_000_000,
            };
            repo.git(&["init", "-q", "-b", "main"]);
            repo
        }

        fn git(&self, args: &[&str]) -> String {
            let out = Command::new("git")
                .args(args)
                .current_dir(self.dir.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_EMAIL", "a@example.com")
                .env("GIT_COMMITTER_NAME", "Committer")
                .env("GIT_COMMITTER_EMAIL", "c@example.com")
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
            String::from_utf8(out.stdout).unwrap()
        }

        fn commit(&mut self, author: &str, path: &str, subject: &str) -> String {
            let file = self.dir.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let before = fs::read_to_string(&file).unwrap_or_default();
            fs::write(&file, format!("{before}{subject}\n")).unwrap();
            self.git(&["add", path]);
            self.clock += 60;
            let date = format!("@{} +0000", self.clock);
            let out = Command::new("git")
                .args(["commit", "-q", "-m", subject])
                .current_dir(self.dir.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", author)
                .env("GIT_AUTHOR_EMAIL", "a@example.com")
                .env("GIT_COMMITTER_NAME", "Committer")
                .env("GIT_COMMITTER_EMAIL", "c@example.com")
                .env("GIT_AUTHOR_DATE", &date)
                .env("GIT_COMMITTER_DATE", &date)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
            self.git(&["rev-parse", "HEAD"]).trim().to_string()
        }

        fn tag(&mut self, name: &str) {
            self.clock += 60;
            let date = format!("@{} +0000", self.clock);
            let out = Command::new("git")
                .args(["tag", "-a", "-m", name, name])
                .current_dir(self.dir.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_COMMITTER_NAME", "Committer")
                .env("GIT_COMMITTER_EMAIL", "c@example.com")
                .env("GIT_COMMITTER_DATE", &date)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
        }
    }

    fn args(attrs: &str) -> Result<GitArgs, String> {
        let text = format!("<!-- computed git {attrs} -->\n<!-- /computed -->\n");
        let file = marker::parse(&text).map_err(|e| e.message)?;
        match &file.segments[0] {
            Segment::Region(r) => GitArgs::from_opener(&r.opener).map_err(|e| format!("{e:?}")),
            Segment::Prose(_) => unreachable!(),
        }
    }

    fn load_in(root: &Path, attrs: &str) -> Result<Loaded, LoadError> {
        load(
            &Ctx::for_template(&root.join("README.md")),
            &args(attrs).unwrap(),
        )
    }

    #[test]
    fn the_opener_names_exactly_one_query() {
        assert_eq!(
            args("log src=src n=3").unwrap().query,
            Query::Log {
                src: "src".into(),
                n: 3
            }
        );
        assert_eq!(args("tags").unwrap().query, Query::Tags { n: 10 });
        assert_eq!(
            args("contributors").unwrap().query,
            Query::Contributors { src: ".".into() }
        );
        for (attrs, message) in [
            ("", "git needs one of log, tags or contributors"),
            ("log tags", "git takes one query, not log and tags"),
            (
                "tags src=x",
                "git tags takes no src=: tags belong to the repository",
            ),
            ("log n=0", "n=0: expected a whole number of at least 1"),
            ("log n=x", "n=x: expected a whole number of at least 1"),
            (
                "contributors n=2",
                "git contributors lists every author; n= is for log and tags",
            ),
            ("blame", "unknown flag \"blame\" for loader git"),
        ] {
            assert_eq!(args(attrs).unwrap_err(), message, "{attrs}");
        }
    }

    #[test]
    fn log_lists_the_commits_touching_a_path_newest_first() {
        let mut repo = Repo::new();
        let first = repo.commit("Ada", "src/a.rs", "Add a");
        repo.commit("Bob", "docs/x.md", "Write docs");
        let third = repo.commit("Ada", "src/b.rs", "Add b");
        let root = repo.dir.path();
        let loaded = load_in(root, "log src=src").unwrap();
        assert_eq!(
            loaded.text,
            format!("- {} Add b\n- {} Add a\n", &third[..7], &first[..7])
        );
        assert_eq!(loaded.snapshot, format!("{third}\n{first}\n").into_bytes());
        assert_eq!(
            load_in(root, "log n=1").unwrap().text,
            format!("- {} Add b\n", &third[..7])
        );
    }

    #[test]
    fn tags_are_newest_first_and_contributors_in_order_of_first_commit() {
        let mut repo = Repo::new();
        repo.commit("Ada", "a", "One");
        repo.tag("v0.1.0");
        repo.commit("Bob", "a", "Two");
        repo.tag("v0.2.0");
        repo.commit("Ada", "b", "Three");
        repo.commit("Cy", "a", "Four");
        repo.tag("v0.10.0");
        let root = repo.dir.path();
        assert_eq!(
            load_in(root, "tags n=2").unwrap().text,
            "- v0.10.0\n- v0.2.0\n"
        );
        let loaded = load_in(root, "contributors").unwrap();
        assert_eq!(loaded.text, "- Ada\n- Bob\n- Cy\n");
        assert_eq!(loaded.snapshot, b"Ada\nBob\nCy\n");
        assert_eq!(load_in(root, "contributors src=b").unwrap().text, "- Ada\n");
    }

    #[test]
    fn outside_a_repository_and_in_a_shallow_clone_the_loader_cannot_answer() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            load_in(dir.path(), "log"),
            Err(LoadError::Hard(m)) if m.contains("not inside a git repository")
        ));
        let mut repo = Repo::new();
        repo.commit("Ada", "a", "One");
        repo.commit("Ada", "a", "Two");
        let clone = tempfile::tempdir().unwrap();
        let url = format!("file://{}", repo.dir.path().display());
        let dest = clone.path().join("c");
        repo.git(&["clone", "-q", "--depth", "1", &url, dest.to_str().unwrap()]);
        assert!(matches!(
            load_in(&dest, "log"),
            Err(LoadError::Hard(m)) if m.contains("shallow clone")
        ));
        assert!(matches!(
            load_in(repo.dir.path(), "log src=missing"),
            Err(LoadError::Hard(m)) if m.contains("missing")
        ));
    }

    #[test]
    fn a_pathspec_is_literal() {
        let mut repo = Repo::new();
        let star = repo.commit("Ada", "a*", "Star");
        repo.commit("Ada", "ab", "Plain");
        let root = repo.dir.path();
        assert_eq!(
            load_in(root, "log src=a*").unwrap().text,
            format!("- {} Star\n", &star[..7])
        );
    }
}
