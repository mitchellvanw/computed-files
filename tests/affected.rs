//! `affected` and `graph` end to end: which regions a path reaches, and the
//! templates, regions and inputs drawn as a graph.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;

const CLAUDE: &str = "# Notes\n\n<!-- computed tree src=src depth=2 name=layout -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"cat docs/*.md\" inputs=docs/*.md name=docs -->\n<!-- /computed -->\n\n<!-- computed file src=README.md name=readme -->\n<!-- /computed -->\n\n<!-- computed exec cmd=date volatile name=clock -->\n<!-- /computed -->\n";

const INDEX: &str = "<!-- computed exec cmd=\"grep -c computed ../CLAUDE.md\" inputs=../CLAUDE.md name=count -->\n<!-- /computed -->\n";

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join(".git")).unwrap();
    fs::create_dir_all(r.join("src/deep/er")).unwrap();
    fs::create_dir_all(r.join("docs")).unwrap();
    fs::create_dir_all(r.join("guide")).unwrap();
    fs::write(r.join("src/main.rs"), "").unwrap();
    fs::write(r.join("docs/a.md"), "# A\n").unwrap();
    fs::write(r.join("docs/b.md"), "# B\n").unwrap();
    fs::write(r.join("README.md"), "Readme\n").unwrap();
    fs::write(r.join("CLAUDE.md"), CLAUDE).unwrap();
    fs::write(r.join("guide/index.md"), INDEX).unwrap();
    dir
}

fn computed(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("computed")
        .unwrap()
        .current_dir(dir)
        .env("XDG_CONFIG_HOME", dir.join(".git/config-home"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The `path:line name` pairs `affected` listed, in order.
fn listed(dir: &Path, paths: &[&str]) -> Vec<String> {
    let mut args = vec!["affected"];
    args.extend(paths);
    let out = computed(dir, &args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout(&out)
        .lines()
        .map(|l| l.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
        .collect()
}

#[test]
fn a_file_an_input_reads_affects_its_region() {
    let dir = repo();
    let d = dir.path();
    assert_eq!(listed(d, &["docs/a.md"]), ["CLAUDE.md:6 docs"]);
    assert_eq!(listed(d, &["README.md"]), ["CLAUDE.md:9 readme"]);
    assert_eq!(listed(d, &["CLAUDE.md"]), ["guide/index.md:1 count"]);
    assert_eq!(
        listed(d, &["README.md", "docs/b.md"]),
        ["CLAUDE.md:6 docs", "CLAUDE.md:9 readme"]
    );
    assert!(listed(d, &["Cargo.toml"]).is_empty());
}

#[test]
fn a_directory_affects_every_region_reading_under_it() {
    let dir = repo();
    let d = dir.path();
    assert_eq!(listed(d, &["docs"]), ["CLAUDE.md:6 docs"]);
    assert_eq!(
        listed(d, &["."]),
        [
            "CLAUDE.md:3 layout",
            "CLAUDE.md:6 docs",
            "CLAUDE.md:9 readme",
            "guide/index.md:1 count"
        ],
        "a volatile region reads nothing"
    );
}

#[test]
fn a_new_or_removed_path_affects_the_regions_that_would_list_it() {
    let dir = repo();
    let d = dir.path();
    assert_eq!(listed(d, &["src/lib.rs"]), ["CLAUDE.md:3 layout"]);
    assert_eq!(listed(d, &["src/deep/new.rs"]), ["CLAUDE.md:3 layout"]);
    assert!(
        listed(d, &["src/deep/er/too-deep.rs"]).is_empty(),
        "depth=2 does not list it"
    );
    assert!(
        listed(d, &["src/.hidden"]).is_empty(),
        "a tree without `all` does not list dotfiles"
    );
    assert_eq!(listed(d, &["docs/new.md"]), ["CLAUDE.md:6 docs"]);
    assert!(listed(d, &["docs/new.txt"]).is_empty());
    fs::remove_file(d.join("README.md")).unwrap();
    assert_eq!(listed(d, &["README.md"]), ["CLAUDE.md:9 readme"]);
}

#[test]
fn paths_resolve_against_the_invocation_directory() {
    let dir = repo();
    let out = computed(&dir.path().join("docs"), &["affected", "a.md"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout(&out),
        "",
        "discovery starts at the invocation directory"
    );
    let out = computed(&dir.path().join("guide"), &["affected", "../CLAUDE.md"]);
    assert_eq!(stdout(&out), "index.md:1 count exec\n");
}

#[test]
fn affected_prints_json() {
    let dir = repo();
    let out = computed(dir.path(), &["--format", "json", "affected", "README.md"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stderr, b"");
    assert_eq!(
        stdout(&out),
        "{\"exit\":0,\"errors\":[],\"regions\":[{\"path\":\"CLAUDE.md\",\"line\":9,\"name\":\"readme\",\"loader\":\"file\"}]}\n"
    );
}

#[test]
fn a_parse_error_is_exit_2_and_the_other_files_still_answer() {
    let dir = repo();
    fs::write(dir.path().join("broken.md"), "<!-- computed tree -->\n").unwrap();
    let out = computed(dir.path(), &["affected", "README.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("broken.md:1: opener without closer"));
    assert!(stdout(&out).contains("CLAUDE.md:9 readme file"));
}

#[test]
fn affected_needs_a_path() {
    let dir = repo();
    assert_eq!(computed(dir.path(), &["affected"]).status.code(), Some(2));
}

#[test]
fn graph_draws_templates_regions_inputs_and_settling_edges_as_mermaid() {
    let dir = repo();
    let out = computed(dir.path(), &["graph"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout(&out),
        "\
flowchart LR
  t0[\"CLAUDE.md\"]
  t1[\"guide/index.md\"]
  r0([\"layout · tree\"])
  r1([\"docs · exec\"])
  r2([\"readme · file\"])
  r3([\"clock · exec volatile\"])
  r4([\"count · exec\"])
  i0[/\"src/\"/]
  i1[/\"docs/*.md\"/]
  i2[/\"README.md\"/]
  t0 --> r0
  t0 --> r1
  t0 --> r2
  t0 --> r3
  t1 --> r4
  r0 --> i0
  r1 --> i1
  r2 --> i2
  r4 -.->|settles| t0
"
    );
}

#[test]
fn graph_draws_dot_and_json() {
    let dir = repo();
    let out = computed(dir.path(), &["--format", "dot", "graph", "CLAUDE.md"]);
    assert_eq!(out.status.code(), Some(0));
    let dot = stdout(&out);
    assert!(dot.starts_with("digraph computed {\n"), "{dot}");
    assert!(
        dot.contains("  t0 [label=\"CLAUDE.md\", shape=box];\n"),
        "{dot}"
    );
    assert!(dot.contains("  r0 -> i0;\n"), "{dot}");
    assert!(!dot.contains("index.md"), "only the templates named: {dot}");

    let out = computed(dir.path(), &["--format", "json", "graph", "guide"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout(&out),
        "{\"exit\":0,\"errors\":[],\"nodes\":[{\"id\":\"t0\",\"kind\":\"template\",\"label\":\"guide/index.md\"},{\"id\":\"r0\",\"kind\":\"region\",\"label\":\"count · exec\",\"path\":\"guide/index.md\",\"line\":1},{\"id\":\"i0\",\"kind\":\"input\",\"label\":\"CLAUDE.md\"}],\"edges\":[{\"from\":\"t0\",\"to\":\"r0\",\"kind\":\"holds\"},{\"from\":\"r0\",\"to\":\"i0\",\"kind\":\"reads\"}]}\n"
    );
}

#[test]
fn graph_formats_belong_to_graph() {
    let dir = repo();
    let out = computed(dir.path(), &["--format", "dot", "check"]);
    assert_eq!(out.status.code(), Some(2));
}
