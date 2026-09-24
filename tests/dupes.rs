//! `dupes` end to end: verbatim blocks copied between Markdown files, and
//! the region that would replace each copy.

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

const INSTALL: &str =
    "## Install\n\nRun the installer.\nThen add it to PATH.\n\nCheck the version.\nYou are done.\n";

const README: &str = "# Tool\n\nIntro.\n\n## Install\n\nRun the installer.\nThen add it to PATH.\n\nCheck the version.\nYou are done.\n\n## Usage\n\nUse it.\n";

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::create_dir_all(d.join(".git")).unwrap();
    fs::write(d.join("README.md"), README).unwrap();
    dir
}

#[test]
fn a_copied_section_is_found_with_the_region_that_includes_it() {
    let dir = repo();
    let d = dir.path();
    fs::write(
        d.join("CLAUDE.md"),
        format!("# Agents\n\n{INSTALL}\n## Rules\n\nBe kind.\n"),
    )
    .unwrap();
    let out = computed(d, &["dupes"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout(&out),
        "5 lines in 2 places\n    README.md:5-11 source\n    CLAUDE.md:3-9 <!-- computed file src=README.md section=Install -->\n"
    );
}

#[test]
fn a_copy_that_is_not_one_section_is_suggested_by_lines_and_paths_are_relative() {
    let dir = repo();
    let d = dir.path();
    fs::create_dir_all(d.join("docs/guide")).unwrap();
    fs::write(
        d.join("docs/guide/setup.md"),
        "# Setup\n\nRun the installer.\nThen add it to PATH.\n\nCheck the version.\nYou are done.\n\nMore.\n",
    )
    .unwrap();
    let out = computed(d, &["dupes"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout(&out),
        "4 lines in 2 places\n    README.md:7-11 source\n    docs/guide/setup.md:3-7 <!-- computed file src=../../README.md lines=7-11 -->\n"
    );
}

#[test]
fn claude_md_is_never_the_source_and_min_lines_bounds_what_counts() {
    let dir = repo();
    let d = dir.path();
    fs::remove_file(d.join("README.md")).unwrap();
    fs::write(d.join("AGENTS.md"), INSTALL).unwrap();
    fs::write(d.join("a.md"), format!("# A\n\n{INSTALL}")).unwrap();
    let out = computed(d, &["dupes"]);
    let text = stdout(&out);
    assert!(text.contains("    a.md:3-9 source\n"), "{text}");
    assert!(
        text.contains("    AGENTS.md:1-7 <!-- computed file src=a.md section=Install -->\n"),
        "{text}"
    );
    let out = computed(d, &["dupes", "--min-lines", "6"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "");
}

#[test]
fn region_bodies_and_trivial_lines_are_not_duplicates() {
    let dir = repo();
    let d = dir.path();
    fs::write(d.join("docs.md"), README).unwrap();
    fs::write(
        d.join("docs.md"),
        "<!-- computed file src=README.md -->\n\n# Tool\n\nIntro.\n\n## Install\n\nRun the installer.\nThen add it to PATH.\n\nCheck the version.\nYou are done.\n\n## Usage\n\nUse it.\n\n<!-- /computed -->\n",
    )
    .unwrap();
    fs::write(d.join("a.md"), "---\n```\n```\n---\n| --- |\n---\n").unwrap();
    fs::write(d.join("b.md"), "---\n```\n```\n---\n| --- |\n---\n").unwrap();
    let out = computed(d, &["dupes"]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert_eq!(stdout(&out), "");
}

#[test]
fn a_block_repeated_within_one_file_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::write(
        d.join("notes.md"),
        "one\ntwo\nthree\nfour\n\nbetween\n\none\ntwo\nthree\nfour\n",
    )
    .unwrap();
    let out = computed(d, &["dupes", "notes.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout(&out),
        "4 lines in 2 places\n    notes.md:1-4 source\n    notes.md:8-11 copy\n"
    );
}

#[test]
fn dupes_prints_json() {
    let dir = repo();
    let d = dir.path();
    fs::write(d.join("CLAUDE.md"), INSTALL).unwrap();
    let out = computed(d, &["--format", "json", "dupes"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stderr, b"");
    assert_eq!(
        stdout(&out),
        "{\"exit\":1,\"errors\":[],\"duplicates\":[{\"lines\":5,\"source\":{\"path\":\"README.md\",\"start\":5,\"end\":11},\"copies\":[{\"path\":\"CLAUDE.md\",\"start\":1,\"end\":7,\"suggestion\":\"<!-- computed file src=README.md section=Install -->\"}]}]}\n"
    );
}
