//! The `git` loader end to end: history renders, commits elsewhere leave
//! the region fresh, a commit to its path makes it stale.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;

fn git(root: &Path, args: &[&str], date: u64) -> String {
    let date = format!("@{date} +0000");
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

/// Commits `path`, writing the subject into it unless it exists already.
fn commit(root: &Path, path: &str, subject: &str, date: u64) -> String {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    if !file.exists() || !path.ends_with(".md") {
        fs::write(&file, subject).unwrap();
    }
    git(root, &["add", path], date);
    git(root, &["commit", "-q", "-m", subject], date);
    git(root, &["rev-parse", "--short=7", "HEAD"], date)
        .trim()
        .to_string()
}

fn computed(root: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("computed")
        .unwrap()
        .current_dir(root)
        .env("XDG_CONFIG_HOME", root.join(".git/test-config"))
        // A pre-commit hook runs with these set; the loader must not follow them.
        .env("GIT_DIR", "/nonexistent")
        .env("GIT_INDEX_FILE", "/nonexistent/index")
        .args(args)
        .output()
        .unwrap()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_log_region_goes_stale_only_when_its_path_gets_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    git(r, &["init", "-q", "-b", "main"], 0);
    let one = commit(r, "src/lib.rs", "Add the library", 1_700_000_000);
    let two = commit(r, "src/main.rs", "Add the binary", 1_700_000_100);
    fs::write(
        r.join("CHANGES.md"),
        "# Changes\n\n<!-- computed git log src=src n=5 -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = computed(r, &["run"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let changes = fs::read_to_string(r.join("CHANGES.md")).unwrap();
    assert!(
        changes.contains(&format!(
            "n=5 | do not edit; run computed -->\n\n- {two} Add the binary\n- {one} Add the library\n\n<!-- /computed in="
        )),
        "{changes}"
    );
    assert_eq!(computed(r, &["check"]).status.code(), Some(0));

    commit(r, "CHANGES.md", "Record the changes", 1_700_000_200);
    let out = computed(r, &["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let three = commit(r, "src/lib.rs", "Grow the library", 1_700_000_300);
    let out = computed(r, &["check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("git stale"), "{}", stderr(&out));
    assert_eq!(computed(r, &["run"]).status.code(), Some(1));
    let changes = fs::read_to_string(r.join("CHANGES.md")).unwrap();
    assert!(
        changes.contains(&format!("\n- {three} Grow the library\n- {two}")),
        "{changes}"
    );
}

#[test]
fn a_shallow_clone_is_a_region_error() {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "main"], 0);
    commit(&origin, "a", "One", 1_700_000_000);
    fs::write(
        origin.join("README.md"),
        "<!-- computed git tags -->\n<!-- /computed -->\n",
    )
    .unwrap();
    commit(&origin, "README.md", "Two", 1_700_000_100);
    let url = format!("file://{}", origin.display());
    git(
        dir.path(),
        &["clone", "-q", "--depth", "1", &url, "clone"],
        0,
    );
    let out = computed(&dir.path().join("clone"), &["check"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        stderr(&out).contains("README.md:1  git error\n    git: the repository is a shallow clone"),
        "{}",
        stderr(&out)
    );
}
