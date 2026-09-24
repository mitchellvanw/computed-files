//! The `file` loader with a slice: `lines=`, `section=` or `anchor=`.

mod sandbox;

use sandbox::{Sandbox, code, hard, parse_error, stderr};

const GUIDE: &str =
    "# Guide\n\nIntro.\n\n## Install\n\nRun `cargo install computed`.\n\n## Use\n\nRun it.\n";

#[test]
fn a_slice_is_the_text_and_the_whole_snapshot() {
    let s = Sandbox::new();
    s.write("docs/guide.md", GUIDE);
    let loaded = s
        .load(
            "README.md",
            "<!-- computed file src=docs/guide.md section=Install -->",
        )
        .unwrap();
    assert_eq!(
        loaded.text,
        "## Install\n\nRun `cargo install computed`.\n\n"
    );
    assert_eq!(
        loaded.snapshot,
        b"docs/guide.md#section=Install\x0043\x00## Install\n\nRun `cargo install computed`.\n\n\x00"
    );
    let loaded = s
        .load(
            "README.md",
            "<!-- computed file src=docs/guide.md lines=3-3 -->",
        )
        .unwrap();
    assert_eq!(loaded.text, "Intro.\n");
    assert_eq!(
        loaded.snapshot,
        b"docs/guide.md#lines=3-3\x007\x00Intro.\n\x00"
    );
}

#[test]
fn an_anchor_takes_the_lines_between_its_markers() {
    let s = Sandbox::new();
    s.write(
        "src/main.rs",
        "fn main() {\n    // ANCHOR: greet\n    println!(\"hi\");\n    // ANCHOR_END: greet\n}\n",
    );
    let loaded = s
        .load(
            "README.md",
            "<!-- computed file src=src/main.rs anchor=greet as=fence lang=rust -->",
        )
        .unwrap();
    assert_eq!(loaded.text, "    println!(\"hi\");\n");
}

#[test]
fn an_unsliced_file_snapshots_as_it_always_did() {
    let s = Sandbox::new();
    s.write("docs/guide.md", "Shared.\n");
    let snap = s
        .snapshot("README.md", "<!-- computed file src=docs/guide.md -->")
        .unwrap();
    assert_eq!(snap, b"docs/guide.md\x008\x00Shared.\n\x00");
}

#[test]
fn a_slice_that_finds_nothing_is_a_hard_error() {
    let s = Sandbox::new();
    s.write("docs/guide.md", GUIDE);
    for (opener, needle) in [
        (
            "<!-- computed file src=docs/guide.md section=Missing -->",
            "src=: docs/guide.md: section=Missing: no heading reads \"Missing\"",
        ),
        (
            "<!-- computed file src=docs/guide.md lines=10-20 -->",
            "the file has 11 lines",
        ),
        (
            "<!-- computed file src=docs/guide.md anchor=x -->",
            "no line holds ANCHOR: x",
        ),
    ] {
        let m = hard(s.snapshot("README.md", opener));
        assert!(m.contains(needle), "{opener}: {m}");
    }
}

#[test]
fn the_grammar_takes_one_slice_and_checks_it() {
    for (opener, needle) in [
        (
            "<!-- computed file src=a.md section=A lines=1-2 -->",
            "section= and lines= both narrow the file",
        ),
        (
            "<!-- computed file src=a.md lines=3-1 -->",
            "ends before it starts",
        ),
        (
            "<!-- computed file src=a.md anchor=\"a b\" -->",
            "anchor=a b",
        ),
        (
            "<!-- computed file src=a.toml key=a -->",
            "unknown attribute key= for loader file",
        ),
    ] {
        let m = parse_error(opener);
        assert!(m.contains(needle), "{opener}: {m}");
    }
}

#[test]
fn an_edit_outside_the_slice_leaves_the_region_fresh() {
    let s = Sandbox::new();
    s.write("docs/guide.md", GUIDE).write(
        "README.md",
        "# Readme\n\n<!-- computed file src=docs/guide.md section=Install -->\n<!-- /computed -->\n",
    );
    let out = s.run(&["run"]);
    assert_eq!(
        code(&out),
        Some(1),
        "a write fails the hook: {}",
        stderr(&out)
    );
    assert!(
        s.read("README.md")
            .contains("Run `cargo install computed`.")
    );

    s.write("docs/guide.md", &GUIDE.replace("Run it.", "Run it often."));
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));

    s.write(
        "docs/guide.md",
        &GUIDE.replace("cargo install computed", "brew install computed"),
    );
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(1));
    assert!(stderr(&out).contains("file stale"), "{}", stderr(&out));
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(s.read("README.md").contains("brew install computed"));
    assert_eq!(code(&s.run(&["run"])), Some(0));
}
