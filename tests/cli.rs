//! The five commands end to end: exit tiers, discovery and the pre-commit scenario.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;

const TEMPLATE: &str = "# Notes\n\n## Layout\n\n<!-- computed tree src=. depth=2 name=layout -->\n<!-- /computed -->\n\n## Decisions\n\n<!-- computed exec cmd=\"grep -h '^# ' docs/adr/*.md\" inputs=docs/adr/*.md name=adrs -->\n<!-- /computed -->\n";

struct Repo {
    dir: tempfile::TempDir,
    config: tempfile::TempDir,
}

impl Repo {
    fn new() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::create_dir_all(r.join("src")).unwrap();
        fs::create_dir_all(r.join("docs/adr")).unwrap();
        fs::create_dir_all(r.join("target")).unwrap();
        fs::write(r.join(".gitignore"), "target/\n").unwrap();
        fs::write(r.join("src/main.rs"), "").unwrap();
        fs::write(r.join("docs/adr/0001-one.md"), "# One\n").unwrap();
        fs::write(
            r.join("target/ignored.md"),
            "<!-- computed tree -->\n<!-- /computed -->\n",
        )
        .unwrap();
        fs::write(r.join("CLAUDE.md"), TEMPLATE).unwrap();
        fs::write(r.join("README.md"), "no regions here\n").unwrap();
        Repo {
            dir,
            config: tempfile::tempdir().unwrap(),
        }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::cargo_bin("computed").unwrap();
        c.current_dir(self.path())
            .env("XDG_CONFIG_HOME", self.config.path())
            .args(args);
        c
    }
    fn claude(&self) -> String {
        fs::read_to_string(self.path().join("CLAUDE.md")).unwrap()
    }
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_pre_commit_scenario() {
    let repo = Repo::new();
    // Untrusted clone: the tree renders, the exec region is skipped, exit 1.
    let out = repo.cmd(&["run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("CLAUDE.md:5 layout tree unrendered") && err.contains("written"),
        "{err}"
    );
    assert!(
        err.contains("CLAUDE.md:10 adrs")
            && err.contains("untrusted")
            && err.contains("computed trust"),
        "{err}"
    );
    assert!(repo.claude().contains("└── src"));
    assert!(
        !repo.claude().contains("target"),
        "the root .gitignore governs the tree: {}",
        repo.claude()
    );
    assert!(!repo.claude().contains("# One"));

    // Trust, then run writes the exec region and exits 1; a second run is a no-op.
    let out = repo.cmd(&["trust"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        stdout(&out).trim(),
        repo.path().canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(repo.cmd(&["run"]).output().unwrap().status.code(), Some(1));
    assert!(repo.claude().contains("# One"));
    let out = repo.cmd(&["run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr(&out), "");
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr(&out), "");
    let out = repo.cmd(&["-v", "check"]).output().unwrap();
    assert!(
        stderr(&out).contains("layout tree fresh"),
        "{}",
        stderr(&out)
    );

    // A new file under src/ makes the layout stale.
    fs::write(repo.path().join("src/lib.rs"), "").unwrap();
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("layout tree stale"),
        "{}",
        stderr(&out)
    );
    assert!(!stderr(&out).contains("adrs"));

    // Dry run shows the diff on stdout and writes nothing.
    let before = repo.claude();
    let out = repo.cmd(&["run", "--dry-run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).contains("+    └── lib.rs") || stdout(&out).contains("+    ├── lib.rs"),
        "{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).starts_with("--- CLAUDE.md"),
        "{}",
        stdout(&out)
    );
    assert_eq!(repo.claude(), before);
    assert_eq!(repo.cmd(&["run"]).output().unwrap().status.code(), Some(1));
    assert!(repo.claude().contains("lib.rs"));
    assert_eq!(
        repo.cmd(&["check"]).output().unwrap().status.code(),
        Some(0)
    );

    // A hand edit is refused, --force overwrites.
    let edited = repo.claude().replace("# One", "# One, edited");
    fs::write(repo.path().join("CLAUDE.md"), &edited).unwrap();
    let out = repo.cmd(&["run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("adrs   exec edited") || stderr(&out).contains("adrs exec edited"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("refused; run with --force"));
    assert_eq!(repo.claude(), edited);
    assert_eq!(
        repo.cmd(&["run", "--force"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        repo.cmd(&["check"]).output().unwrap().status.code(),
        Some(0)
    );

    // Untrust again: --trust is the one-shot grant.
    assert!(repo.cmd(&["untrust"]).output().unwrap().status.success());
    fs::write(repo.path().join("docs/adr/0002-two.md"), "# Two\n").unwrap();
    let out = repo.cmd(&["run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("untrusted"));
    assert_eq!(
        repo.cmd(&["run", "--trust"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert!(repo.claude().contains("# Two"));
    assert_eq!(
        repo.cmd(&["check"]).output().unwrap().status.code(),
        Some(0)
    );

    // Clean, then check reports both unrendered, then run restores.
    assert_eq!(
        repo.cmd(&["clean"]).output().unwrap().status.code(),
        Some(1)
    );
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out).matches("unrendered").count(),
        2,
        "{}",
        stderr(&out)
    );
    assert!(!repo.claude().contains("# Two"));
    assert_eq!(
        repo.cmd(&["run", "--trust"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        repo.cmd(&["check"]).output().unwrap().status.code(),
        Some(0)
    );

    // clean --dry-run prints the diff and writes nothing.
    let before = repo.claude();
    let out = repo.cmd(&["clean", "--dry-run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout(&out).contains("-└── src"), "{}", stdout(&out));
    assert!(stderr(&out).contains("would clean"), "{}", stderr(&out));
    assert_eq!(repo.claude(), before);

    // A one-sum closer is a parse error: exit 2, nothing written.
    let broken = repo.claude().replacen(" out=", " xout=", 1);
    fs::write(repo.path().join("CLAUDE.md"), &broken).unwrap();
    let out = repo.cmd(&["run", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("CLAUDE.md:"), "{}", stderr(&out));
    assert_eq!(repo.claude(), broken);
}

/// The ignore state is part of what the tree is computed from: an edit to
/// `.gitignore` that changes the listing drifts the region like any other
/// input, and one that does not change the listing is not drift.
#[test]
fn editing_gitignore_drifts_the_tree_region() {
    let repo = Repo::new();
    fs::write(repo.path().join("src/build.log"), "").unwrap();
    assert_eq!(repo.cmd(&["run"]).output().unwrap().status.code(), Some(1));
    assert!(repo.claude().contains("build.log"));
    let out = repo.cmd(&["check"]).output().unwrap();
    assert!(!stderr(&out).contains("layout"), "{}", stderr(&out));

    fs::write(
        repo.path().join(".gitignore"),
        "# build\n\ntarget/\n*.log\n",
    )
    .unwrap();
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("layout tree stale"),
        "{}",
        stderr(&out)
    );
    assert_eq!(repo.cmd(&["run"]).output().unwrap().status.code(), Some(1));
    assert!(!repo.claude().contains("build.log"), "{}", repo.claude());
    let out = repo.cmd(&["check"]).output().unwrap();
    assert!(!stderr(&out).contains("layout"), "{}", stderr(&out));

    // A comment-only edit leaves the listing, and so the region, as it was.
    fs::write(
        repo.path().join(".gitignore"),
        "# logs too\ntarget/\n*.log\n",
    )
    .unwrap();
    let out = repo.cmd(&["check"]).output().unwrap();
    assert!(!stderr(&out).contains("layout"), "{}", stderr(&out));

    // `gitignore` is not a flag: ignore rules always apply inside a repository.
    let flagged = repo
        .claude()
        .replace("depth=2 name=layout", "depth=2 gitignore name=layout");
    fs::write(repo.path().join("CLAUDE.md"), flagged).unwrap();
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("unknown flag \"gitignore\""),
        "{}",
        stderr(&out)
    );
}

#[test]
fn discovery_reads_md_files_and_skips_ignored_ones() {
    let repo = Repo::new();
    fs::create_dir_all(repo.path().join("docs/sub")).unwrap();
    fs::write(
        repo.path().join("docs/sub/guide.md"),
        "<!-- computed tree src=. name=here -->\n<!-- /computed -->\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("notes.txt"),
        "<!-- computed tree -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = repo.cmd(&["check"]).output().unwrap();
    let err = stderr(&out);
    assert!(err.contains("CLAUDE.md:5"), "{err}");
    assert!(err.contains("docs/sub/guide.md:1 here"), "{err}");
    assert!(!err.contains("target/"), "{err}");
    assert!(!err.contains("notes.txt"), "{err}");
    assert!(!err.contains("README.md"), "{err}");
    // Explicit paths: a file of any extension, a directory, and a missing path.
    let out = repo.cmd(&["check", "notes.txt"]).output().unwrap();
    assert!(stderr(&out).contains("notes.txt:1"), "{}", stderr(&out));
    let out = repo.cmd(&["check", "docs"]).output().unwrap();
    assert!(
        stderr(&out).contains("docs/sub/guide.md:1") && !stderr(&out).contains("CLAUDE.md"),
        "{}",
        stderr(&out)
    );
    let out = repo.cmd(&["check", "missing.md"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn usage_errors_exit_2() {
    let repo = Repo::new();
    assert_eq!(
        repo.cmd(&["check", "--force"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        repo.cmd(&["check", "--trust"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        repo.cmd(&["clean", "--trust"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        repo.cmd(&["bogus"]).output().unwrap().status.code(),
        Some(2)
    );
    assert!(repo.cmd(&["--version"]).output().unwrap().status.success());
}

#[test]
fn a_loader_failure_keeps_the_body_and_prints_stderr_under_the_region() {
    let repo = Repo::new();
    fs::write(
        repo.path().join("CLAUDE.md"),
        "<!-- computed exec cmd=\"echo boom >&2; false\" volatile name=bad -->\n<!-- /computed -->\n<!-- computed tree name=ok -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = repo.cmd(&["run", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("bad exec unrendered") || err.contains("bad  exec unrendered"),
        "{err}"
    );
    assert!(err.contains("\n    boom"), "{err}");
    let text = repo.claude();
    assert!(
        text.contains("<!-- computed tree name=ok | do not edit"),
        "{text}"
    );
    assert!(text.starts_with("<!-- computed exec cmd=\"echo boom >&2; false\" volatile name=bad -->\n<!-- /computed -->\n"), "{text}");
}

#[test]
fn a_tier_2_file_is_skipped_and_the_others_are_still_written() {
    let repo = Repo::new();
    fs::write(
        repo.path().join("broken.md"),
        "<!-- computed tree src=../../.. -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = repo.cmd(&["run", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("broken.md:1"), "{}", stderr(&out));
    assert!(repo.claude().contains("└── src"));
}

#[test]
fn a_symlinked_template_stays_a_symlink_and_is_rendered_once() {
    let repo = Repo::new();
    std::os::unix::fs::symlink("CLAUDE.md", repo.path().join("AGENTS.md")).unwrap();
    let out = repo.cmd(&["run", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let link = fs::symlink_metadata(repo.path().join("AGENTS.md")).unwrap();
    assert!(link.file_type().is_symlink());
    assert!(repo.claude().contains("# One"));
    assert!(!stderr(&out).contains("AGENTS.md"), "{}", stderr(&out));
    let out = repo.cmd(&["run", "--trust", "AGENTS.md"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        fs::symlink_metadata(repo.path().join("AGENTS.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn computed_file_names_the_template_from_any_directory() {
    let repo = Repo::new();
    fs::write(
        repo.path().join("docs/guide.md"),
        "<!-- computed exec cmd=\"head -c 11 < $COMPUTED_FILE\" volatile -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = repo
        .cmd(&["run", "--trust", "docs/guide.md"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let text = fs::read_to_string(repo.path().join("docs/guide.md")).unwrap();
    assert!(text.contains("\n<!-- comput\n"), "{text}");
}

#[test]
fn a_markdown_file_that_is_not_utf8_is_skipped_unless_it_has_markers() {
    let repo = Repo::new();
    fs::write(repo.path().join("latin1.md"), b"caf\xe9\n").unwrap();
    let out = repo.cmd(&["run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(!stderr(&out).contains("latin1"), "{}", stderr(&out));
    fs::write(
        repo.path().join("latin1.md"),
        b"caf\xe9\n<!-- computed tree -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = repo.cmd(&["check", "latin1.md"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("latin1.md: not UTF-8"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn templates_that_read_each_other_settle_in_one_run() {
    let repo = Repo::new();
    let r = repo.path();
    fs::write(r.join("CLAUDE.md"), "").unwrap();
    // `a.md` sorts first and reads `b.md`, which changes after it.
    fs::write(
        r.join("a.md"),
        "<!-- computed exec cmd=\"grep -c computed b.md\" inputs=b.md -->\n<!-- /computed -->\n",
    )
    .unwrap();
    fs::write(
        r.join("b.md"),
        "<!-- computed exec cmd=\"echo b\" inputs=a.md -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = repo.cmd(&["run", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = repo.cmd(&["run", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn a_render_that_feeds_itself_is_an_error_not_a_loop() {
    let repo = Repo::new();
    let r = repo.path();
    fs::write(r.join("CLAUDE.md"), "").unwrap();
    // Each render adds a line to what the other reads: no fixed point.
    fs::write(
        r.join("a.md"),
        "<!-- computed exec cmd=\"grep '^x' b.md; echo x\" inputs=b.md -->\n<!-- /computed -->\n",
    )
    .unwrap();
    fs::write(
        r.join("b.md"),
        "<!-- computed exec cmd=\"grep '^x' a.md; echo x\" inputs=a.md -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = repo.cmd(&["run", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("still changing"), "{}", stderr(&out));
}

#[test]
fn discovery_includes_dot_directories_and_markdown_extension() {
    let repo = Repo::new();
    let r = repo.path();
    fs::create_dir_all(r.join(".claude/skills/x")).unwrap();
    fs::create_dir_all(r.join(".github")).unwrap();
    let region = "<!-- computed tree src=. -->\n<!-- /computed -->\n";
    fs::write(r.join(".claude/skills/x/SKILL.md"), region).unwrap();
    fs::write(r.join(".github/notes.markdown"), region).unwrap();
    fs::write(r.join(".git/COMMIT.md"), region).unwrap();
    let err = stderr(&repo.cmd(&["check"]).output().unwrap());
    assert!(err.contains(".claude/skills/x/SKILL.md:1"), "{err}");
    assert!(err.contains(".github/notes.markdown:1"), "{err}");
    assert!(!err.contains(".git/"), "{err}");
}

#[test]
fn only_narrows_to_named_regions_and_an_unknown_name_is_an_error() {
    let repo = Repo::new();
    let out = repo.cmd(&["run", "--only", "layout"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(!stderr(&out).contains("adrs"), "{}", stderr(&out));
    assert!(repo.claude().contains("└── src"));
    let out = repo.cmd(&["check", "--only", "nope"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("no region is named \"nope\""),
        "{}",
        stderr(&out)
    );
}

#[test]
fn json_reports_every_region_on_stdout() {
    let repo = Repo::new();
    let out = repo.cmd(&["--format", "json", "check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "");
    let doc = stdout(&out);
    assert!(
        doc.starts_with("{\"exit\":1,\"files\":[{\"path\":\"CLAUDE.md\""),
        "{doc}"
    );
    assert!(
        doc.contains(
            "\"name\":\"layout\",\"loader\":\"tree\",\"state\":\"unrendered\",\"action\":null"
        ),
        "{doc}"
    );
    let out = repo
        .cmd(&["run", "--dry-run", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        stdout(&out).contains("\"diff\":\"--- CLAUDE.md"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn dry_run_shows_what_force_would_do_to_a_refused_file() {
    let repo = Repo::new();
    repo.cmd(&["run", "--trust"]).output().unwrap();
    let edited = repo.claude().replace("# One", "# One, edited");
    fs::write(repo.path().join("CLAUDE.md"), &edited).unwrap();
    let out = repo.cmd(&["run", "--dry-run", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("refused"), "{}", stderr(&out));
    let diff = stdout(&out);
    assert!(diff.contains("+++ CLAUDE.md (run --force)"), "{diff}");
    assert!(
        diff.contains("-# One, edited") && diff.contains("+# One"),
        "{diff}"
    );
    assert_eq!(repo.claude(), edited);
}
