//! `computed stats`: the size of every region and the share of each file
//! that is computed, read from the files alone.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use assert_cmd::prelude::*;

fn stats(root: &Path, args: &[&str]) -> Output {
    let mut c = Command::cargo_bin("computed").unwrap();
    c.current_dir(root)
        .env("XDG_CONFIG_HOME", root.join(".config"))
        .arg("stats")
        .args(args);
    c.output().unwrap()
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join(".git")).unwrap();
    fs::create_dir_all(r.join("docs")).unwrap();
    // 12 bytes of prose, a region of 3 lines and 10 bytes, a region of none.
    fs::write(
        r.join("README.md"),
        "# Read me\n\n\n<!-- computed exec cmd=\"touch ran\" volatile name=now -->\n\nabcdefg\n\n<!-- /computed -->\n<!-- computed tree -->\n<!-- /computed -->\n",
    )
    .unwrap();
    fs::write(
        r.join("docs/guide.md"),
        "<!-- computed file src=../README.md name=readme -->\nx\n<!-- /computed -->\n",
    )
    .unwrap();
    fs::write(r.join("docs/plain.md"), "no regions\n").unwrap();
    dir
}

fn text(out: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn stats_lists_regions_by_path_and_line_with_a_line_per_file_and_totals() {
    let dir = repo();
    let out = stats(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let (stdout, stderr) = text(&out);
    assert_eq!(stderr, "");
    let readme = fs::metadata(dir.path().join("README.md")).unwrap().len();
    let guide = fs::metadata(dir.path().join("docs/guide.md"))
        .unwrap()
        .len();
    let total = readme + guide;
    let pct = |part: u64, whole: u64| format!("{:.1}%", part as f64 * 100.0 / whole as f64);
    let tok = |b: u64| b.div_ceil(4);
    let expected = format!(
        "\
README.md:4 now exec 3 lines 10 bytes ~3 tokens
README.md:9     tree 0 lines 0 bytes ~0 tokens
README.md: 10 of {readme} bytes computed ({}), ~3 of ~{} tokens
docs/guide.md:1 readme file 1 line 2 bytes ~1 token
docs/guide.md: 2 of {guide} bytes computed ({}), ~1 of ~{} tokens
3 regions in 2 files: 12 of {total} bytes computed ({}), ~3 of ~{} tokens
",
        pct(10, readme),
        tok(readme),
        pct(2, guide),
        tok(guide),
        pct(12, total),
        tok(total),
    );
    assert_eq!(stdout, expected);
    assert!(
        !dir.path().join("ran").exists(),
        "stats never runs a loader"
    );
}

#[test]
fn stats_takes_paths_and_prints_json() {
    let dir = repo();
    let out = stats(dir.path(), &["docs", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let (stdout, stderr) = text(&out);
    assert_eq!(stderr, "");
    let guide = fs::metadata(dir.path().join("docs/guide.md"))
        .unwrap()
        .len();
    let pct = format!("{:.1}", 2.0 * 100.0 / guide as f64);
    let tokens = guide.div_ceil(4);
    assert_eq!(
        stdout,
        format!(
            "{{\"exit\":0,\"files\":[{{\"path\":\"docs/guide.md\",\"error\":null,\"bytes\":{guide},\"region_bytes\":2,\"percent\":{pct},\"tokens\":{tokens},\"region_tokens\":1,\"regions\":[{{\"line\":1,\"name\":\"readme\",\"loader\":\"file\",\"lines\":1,\"bytes\":2,\"tokens\":1}}]}}],\"totals\":{{\"files\":1,\"regions\":1,\"bytes\":{guide},\"region_bytes\":2,\"percent\":{pct},\"tokens\":{tokens},\"region_tokens\":1}}}}\n"
        )
    );
}

#[test]
fn a_file_that_does_not_parse_is_an_error_and_the_rest_are_counted() {
    let dir = repo();
    fs::write(
        dir.path().join("docs/broken.md"),
        "<!-- computed tree -->\nno closer\n",
    )
    .unwrap();
    let out = stats(dir.path(), &["docs"]);
    assert_eq!(out.status.code(), Some(2));
    let (stdout, stderr) = text(&out);
    assert_eq!(stderr, "docs/broken.md:1: opener without closer\n");
    assert!(stdout.starts_with("docs/guide.md:1 readme"), "{stdout}");
    let guide = fs::metadata(dir.path().join("docs/guide.md"))
        .unwrap()
        .len();
    let totals = format!(
        "1 region in 1 file: 2 of {guide} bytes computed ({:.1}%), ~1 of ~{} tokens\n",
        200.0 / guide as f64,
        guide.div_ceil(4)
    );
    assert!(stdout.ends_with(&totals), "{stdout}");
}
