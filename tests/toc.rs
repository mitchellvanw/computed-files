//! The `toc` loader: the template's own prose headings, which settles in
//! one `run` although the loader reads the file `run` writes.

mod sandbox;

use sandbox::{Sandbox, code, parse_error, stderr};

const README: &str = "# Project\n\n## Contents\n\n<!-- computed toc name=toc -->\n<!-- /computed -->\n\n## Install\n\n<!-- computed file src=docs/install.md name=install -->\n<!-- /computed -->\n\n## Use\n\n```md\n## Not a heading\n```\n\n### Flags\n";

const INSTALL: &str = "## From source\n\nRun `cargo install`.\n\n## Install\n";

#[test]
fn run_writes_the_toc_once_and_a_second_run_is_a_no_op() {
    let s = Sandbox::new();
    s.write("docs/install.md", INSTALL)
        .write("README.md", README);
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    let rendered = s.read("README.md");
    assert!(
        rendered.contains(
            "| do not edit; run computed -->\n\n- [Contents](#contents)\n- [Install](#install)\n- [Use](#use)\n  - [Flags](#flags)\n\n<!-- /computed in="
        ),
        "the included headings are not the template's: {rendered}"
    );
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    assert_eq!(s.read("README.md"), rendered);
    let out = s.run(&["check", "-v"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out)
            .lines()
            .any(|l| l.starts_with("README.md:5 toc") && l.ends_with(" toc fresh")),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_heading_edit_makes_the_toc_stale_and_other_prose_does_not() {
    let s = Sandbox::new();
    s.write("docs/install.md", INSTALL)
        .write("README.md", README);
    s.run(&["run"]);
    let rendered = s.read("README.md");
    s.write(
        "README.md",
        &rendered.replace("### Flags\n", "### Flags\n\nMore prose.\n"),
    );
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    s.write(
        "docs/install.md",
        &INSTALL.replace("## Install", "## Use\n\n# Top"),
    );
    let out = s.run(&["check"]);
    assert!(
        !stderr(&out).contains("toc toc"),
        "an included file's headings are not the toc's: {}",
        stderr(&out)
    );
    s.run(&["run"]);
    let rendered = s.read("README.md");
    s.write("README.md", &rendered.replace("### Flags", "### Options"));
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(1));
    assert!(stderr(&out).contains("toc toc stale"), "{}", stderr(&out));
    assert_eq!(code(&s.run(&["run"])), Some(1));
    assert!(s.read("README.md").contains("  - [Options](#options)\n"));
    assert_eq!(code(&s.run(&["run"])), Some(0));
}

#[test]
fn levels_are_checked() {
    for (opener, needle) in [
        (
            "<!-- computed toc min=0 -->",
            "min=0: expected a heading level",
        ),
        ("<!-- computed toc max=7 -->", "max=7"),
        ("<!-- computed toc min=4 max=3 -->", "min=4 is above max=3"),
        ("<!-- computed toc src=x -->", "unknown attribute src="),
    ] {
        let m = parse_error(opener);
        assert!(m.contains(needle), "{opener}: {m}");
    }
    let s = Sandbox::new();
    s.write(
        "README.md",
        "# A\n## B\n### C\n#### D\n\n<!-- computed toc min=4 -->\n<!-- /computed -->\n",
    );
    let loaded = s.load("README.md", "<!-- computed toc min=4 -->").unwrap();
    assert_eq!(loaded.text, "- [D](#d)\n");
    let loaded = s.load("README.md", "<!-- computed toc max=1 -->").unwrap();
    assert_eq!(loaded.text, "- [A](#a)\n");
}
