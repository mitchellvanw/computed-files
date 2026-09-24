//! `merge` end to end: the driver installed in a real repository, two
//! branches that each re-render a region, and the merge that leaves it
//! unrendered for the next `run`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use assert_cmd::prelude::*;

fn bin_dir() -> PathBuf {
    let bin = Command::cargo_bin("computed").unwrap();
    Path::new(bin.get_program()).parent().unwrap().to_path_buf()
}

/// `PATH` with this build's binary first, as the driver git runs finds it.
fn path_env() -> String {
    format!(
        "{}:{}",
        bin_dir().display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn git_output(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("PATH", path_env())
        .args(args)
        .output()
        .unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_output(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn computed(dir: &Path, args: &[&str]) -> Output {
    Command::cargo_bin("computed")
        .unwrap()
        .current_dir(dir)
        .env("XDG_CONFIG_HOME", dir.join(".git/config-home"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const NOTES: &str =
    "# Notes\n\n<!-- computed tree src=src name=layout -->\n<!-- /computed -->\n\nProse.\n";

/// A repository with a rendered tree region, committed on `main`.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    git(d, &["init", "-q", "-b", "main"]);
    git(d, &["config", "user.name", "Test"]);
    git(d, &["config", "user.email", "test@example.com"]);
    fs::create_dir_all(d.join("src")).unwrap();
    fs::write(d.join("src/main.rs"), "").unwrap();
    fs::write(d.join("NOTES.md"), NOTES).unwrap();
    assert_eq!(computed(d, &["run"]).status.code(), Some(1));
    git(d, &["add", "-A"]);
    git(d, &["commit", "-qm", "Render"]);
    dir
}

/// On a new branch from `main`, adds `file` under src/, re-renders and commits.
fn branch_adding(d: &Path, branch: &str, file: &str) {
    git(d, &["checkout", "-q", "-b", branch, "main"]);
    fs::write(d.join("src").join(file), "").unwrap();
    assert_eq!(computed(d, &["run"]).status.code(), Some(1));
    git(d, &["add", "-A"]);
    git(d, &["commit", "-qm", &format!("Add {file}")]);
}

#[test]
fn install_writes_the_attributes_and_the_local_driver_once() {
    let dir = repo();
    let d = dir.path();
    fs::write(d.join(".gitattributes"), "*.png binary").unwrap();
    let out = computed(d, &["merge", "--install"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout(&out),
        "added `*.md merge=computed` to .gitattributes\n\
         added `*.markdown merge=computed` to .gitattributes\n\
         set merge.computed.name = computed\n\
         set merge.computed.driver = computed merge %O %A %B %P\n"
    );
    assert_eq!(
        fs::read_to_string(d.join(".gitattributes")).unwrap(),
        "*.png binary\n*.md merge=computed\n*.markdown merge=computed\n"
    );
    assert_eq!(
        git(d, &["config", "--local", "merge.computed.driver"]).trim(),
        "computed merge %O %A %B %P"
    );
    let out = computed(d, &["merge", "--install"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "already installed\n");
}

#[test]
fn install_does_not_write_through_a_symlinked_gitattributes() {
    let dir = repo();
    let d = dir.path();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim");
    fs::write(&victim, "keep\n").unwrap();
    std::os::unix::fs::symlink(&victim, d.join(".gitattributes")).unwrap();
    let out = computed(d, &["merge", "--install"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("symlink"),
        "{out:?}"
    );
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep\n");
}

#[test]
fn two_branches_that_re_render_a_region_merge_cleanly_and_run_makes_it_fresh() {
    let dir = repo();
    let d = dir.path();
    assert_eq!(computed(d, &["merge", "--install"]).status.code(), Some(0));
    git(d, &["add", "-A"]);
    git(d, &["commit", "-qm", "Install the merge driver"]);
    branch_adding(d, "a", "a.rs");
    branch_adding(d, "b", "b.rs");
    git(d, &["checkout", "-q", "a"]);
    let out = git_output(d, &["merge", "--no-edit", "b"]);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let notes = fs::read_to_string(d.join("NOTES.md")).unwrap();
    assert!(!notes.contains("<<<<<<<"), "{notes}");
    assert!(notes.contains("\n<!-- /computed -->\n"), "{notes}");
    let out = computed(d, &["check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("layout tree unrendered"));
    assert_eq!(computed(d, &["run"]).status.code(), Some(1));
    assert_eq!(computed(d, &["check"]).status.code(), Some(0));
    let notes = fs::read_to_string(d.join("NOTES.md")).unwrap();
    assert!(notes.contains("├── a.rs\n├── b.rs\n└── main.rs"), "{notes}");
}

#[test]
fn without_the_driver_the_same_merge_conflicts() {
    let dir = repo();
    let d = dir.path();
    branch_adding(d, "a", "a.rs");
    branch_adding(d, "b", "b.rs");
    git(d, &["checkout", "-q", "a"]);
    assert!(!git_output(d, &["merge", "--no-edit", "b"]).status.success());
}

const BASE: &str = "# Title\n\n<!-- computed tree src=src -->\n```\n.\n└── main.rs\n```\n<!-- /computed in=1111111111111111111111111111111111111111111111111111111111111111 out=2222222222222222222222222222222222222222222222222222222222222222 -->\n\nProse.\n";

fn driver(base: &str, ours: &str, theirs: &str) -> (Output, String) {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::write(d.join("O"), base).unwrap();
    fs::write(d.join("A"), ours).unwrap();
    fs::write(d.join("B"), theirs).unwrap();
    let out = computed(d, &["merge", "O", "A", "B", "NOTES.md"]);
    (out, fs::read_to_string(d.join("A")).unwrap())
}

#[test]
fn the_driver_unrenders_a_conflicted_region_and_keeps_a_prose_conflict() {
    let region = |file: &str, sum: char| {
        BASE.replace("└── main.rs", &format!("├── {file}\n└── main.rs"))
            .replace('1', &sum.to_string())
            .replace('2', &sum.to_string())
    };
    let (out, merged) = driver(BASE, &region("a.rs", 'a'), &region("b.rs", 'b'));
    assert_eq!(out.status.code(), Some(0), "{merged}");
    assert_eq!(
        merged,
        "# Title\n\n<!-- computed tree src=src -->\n```\n.\n├── a.rs\n└── main.rs\n```\n<!-- /computed -->\n\nProse.\n"
    );

    let ours = region("a.rs", 'a').replace("Prose.", "Our prose.");
    let theirs = region("b.rs", 'b').replace("Prose.", "Their prose.");
    let (out, merged) = driver(BASE, &ours, &theirs);
    assert_eq!(out.status.code(), Some(1), "{merged}");
    assert!(merged.contains("\n<!-- /computed -->\n"), "{merged}");
    assert!(
        merged.contains("<<<<<<< ours\nOur prose.\n=======\nTheir prose.\n>>>>>>> theirs\n"),
        "{merged}"
    );
}

#[test]
fn a_conflict_in_an_opener_stays_a_conflict() {
    let ours = BASE.replace("tree src=src", "tree src=src depth=1");
    let theirs = BASE.replace("tree src=src", "tree src=src depth=2");
    let (out, merged) = driver(BASE, &ours, &theirs);
    assert_eq!(out.status.code(), Some(1));
    assert!(merged.contains("<<<<<<< ours\n"), "{merged}");
    assert!(merged.contains("in=1111"), "{merged}");
}

#[test]
fn a_clean_merge_is_written_as_it_is() {
    let ours = BASE.replace("# Title", "# Our title");
    let theirs = BASE.replace("Prose.", "Their prose.");
    let (out, merged) = driver(BASE, &ours, &theirs);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        merged,
        BASE.replace("# Title", "# Our title")
            .replace("Prose.", "Their prose.")
    );
}

#[test]
fn merge_needs_three_files_or_install() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(computed(dir.path(), &["merge"]).status.code(), Some(2));
    assert_eq!(
        computed(dir.path(), &["merge", "a", "b"]).status.code(),
        Some(2)
    );
    assert_eq!(
        computed(dir.path(), &["merge", "--install", "a", "b", "c"])
            .status
            .code(),
        Some(2)
    );
}
