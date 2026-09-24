//! `adopt` end to end: a hand edit inside a `file` region written back into
//! its source, and the region fresh again.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use assert_cmd::prelude::*;

fn computed(dir: &Path, args: &[&str]) -> Output {
    Command::cargo_bin("computed")
        .unwrap()
        .current_dir(dir)
        .env("XDG_CONFIG_HOME", dir.join(".git/config-home"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn read(dir: &Path, path: &str) -> String {
    fs::read_to_string(dir.join(path)).unwrap()
}

const README: &str = "# Readme\n\n<!-- computed file src=docs/intro.md name=intro -->\n<!-- /computed -->\n\n- An item\n  <!-- computed file src=src/lib.rs as=fence lang=rust name=code -->\n  <!-- /computed -->\n\n<!-- computed tree src=src name=layout -->\n<!-- /computed -->\n";

/// A rendered README whose regions include `docs/intro.md` and `src/lib.rs`.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::create_dir_all(d.join(".git")).unwrap();
    fs::create_dir_all(d.join("docs")).unwrap();
    fs::create_dir_all(d.join("src")).unwrap();
    fs::write(d.join("docs/intro.md"), "Intro line one\nline two\n").unwrap();
    fs::write(d.join("src/lib.rs"), "fn a() {}\n\nfn b() {}\n").unwrap();
    fs::write(d.join("README.md"), README).unwrap();
    assert_eq!(computed(d, &["run"]).status.code(), Some(1));
    assert_eq!(computed(d, &["check"]).status.code(), Some(0));
    dir
}

fn edit(d: &Path, from: &str, to: &str) {
    let text = read(d, "README.md");
    assert!(text.contains(from), "{text}");
    fs::write(d.join("README.md"), text.replacen(from, to, 1)).unwrap();
}

#[test]
fn a_raw_edit_is_written_back_and_the_region_is_fresh() {
    let dir = repo();
    let d = dir.path();
    edit(d, "\nline two\n", "\nline 2, adopted\n");
    let out = computed(d, &["adopt", "README.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        read(d, "docs/intro.md"),
        "Intro line one\nline 2, adopted\n"
    );
    assert!(
        stderr(&out).contains("README.md:3 intro file edited adopted into docs/intro.md"),
        "{}",
        stderr(&out)
    );
    let out = computed(d, &["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn a_fenced_indented_edit_is_written_back_without_fence_or_indent() {
    let dir = repo();
    let d = dir.path();
    edit(d, "  fn b() {}\n", "  fn b() { todo!() }\n");
    let out = computed(d, &["adopt", "README.md", "--only", "code"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(read(d, "src/lib.rs"), "fn a() {}\n\nfn b() { todo!() }\n");
    assert_eq!(computed(d, &["check"]).status.code(), Some(0));
}

#[test]
fn dry_run_prints_the_source_diff_and_writes_nothing() {
    let dir = repo();
    let d = dir.path();
    edit(d, "\nline two\n", "\nline 2\n");
    let before = read(d, "README.md");
    let out = computed(d, &["adopt", "README.md", "--dry-run"]);
    assert_eq!(out.status.code(), Some(1));
    let diff = stdout(&out);
    assert!(
        diff.starts_with("--- docs/intro.md\n+++ docs/intro.md\n"),
        "{diff}"
    );
    assert!(diff.contains("-line two\n+line 2\n"), "{diff}");
    assert_eq!(read(d, "docs/intro.md"), "Intro line one\nline two\n");
    assert_eq!(read(d, "README.md"), before);
    assert!(stderr(&out).contains("would adopt into docs/intro.md"));
}

#[test]
fn a_computed_loader_cannot_be_adopted() {
    let dir = repo();
    let d = dir.path();
    edit(d, "└── lib.rs", "└── lib.rs, edited");
    let out = computed(d, &["adopt", "README.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("README.md:19 layout tree edited refused; a tree body is computed"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_source_that_changed_since_the_render_is_a_conflict() {
    let dir = repo();
    let d = dir.path();
    edit(d, "\nline two\n", "\nline 2\n");
    fs::write(d.join("docs/intro.md"), "Intro line one\nline II\n").unwrap();
    let out = computed(d, &["adopt", "README.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("stale+edited refused; the source changed since the render"),
        "{}",
        stderr(&out)
    );
    assert_eq!(read(d, "docs/intro.md"), "Intro line one\nline II\n");
}

#[test]
fn an_edit_that_does_not_round_trip_is_refused() {
    let dir = repo();
    let d = dir.path();
    // The raw sink's closing blank line is gone: no source renders to that.
    edit(d, "line two\n\n<!-- /computed", "line 2\n<!-- /computed");
    let before = read(d, "README.md");
    let out = computed(d, &["adopt", "README.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("does not round-trip"),
        "{}",
        stderr(&out)
    );
    assert_eq!(read(d, "docs/intro.md"), "Intro line one\nline two\n");
    assert_eq!(read(d, "README.md"), before);
}

#[test]
fn a_crlf_source_stays_crlf() {
    let dir = repo();
    let d = dir.path();
    fs::write(d.join("docs/intro.md"), "Intro line one\r\nline two\r\n").unwrap();
    assert_eq!(computed(d, &["run"]).status.code(), Some(1));
    edit(d, "\nline two\n", "\nline 2\n");
    let out = computed(d, &["adopt", "README.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(read(d, "docs/intro.md"), "Intro line one\r\nline 2\r\n");
    assert_eq!(computed(d, &["check"]).status.code(), Some(0));
}

#[test]
fn nothing_edited_is_nothing_to_adopt() {
    let dir = repo();
    let out = computed(dir.path(), &["adopt", "README.md"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr(&out), "");
    let out = computed(dir.path(), &["adopt", "README.md", "--only", "nope"]);
    assert_eq!(out.status.code(), Some(2));
}
