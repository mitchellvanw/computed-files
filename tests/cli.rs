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
