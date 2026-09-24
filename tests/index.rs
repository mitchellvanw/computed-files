//! The `index` loader: a link per file a glob selects, titled by its first
//! level-one heading or its file name.

mod sandbox;

use std::path::Path;
use std::process::Command;

use computed::loader::{Ctx, Production};
use computed::render::Loaders;
use sandbox::{Sandbox, code, hard, parse_error, region, stderr};

#[test]
fn one_link_per_file_in_byte_order_titled_by_its_h1() {
    let s = Sandbox::new();
    s.write(
        "docs/adr/0002-b.md",
        "---\nstatus: accepted\n---\n\n# Second\n\nBody.\n",
    )
    .write("docs/adr/0001-a.md", "# First `one`\n")
    .write("docs/adr/0010-c.md", "No heading.\n")
    .write("docs/adr/Z.md", "## Only a level two\n")
    .write("docs/adr/notes.txt", "# not markdown\n");
    let loaded = s
        .load("CLAUDE.md", "<!-- computed index src=\"docs/adr/*.md\" -->")
        .unwrap();
    assert_eq!(
        loaded.text,
        "- [First `one`](docs/adr/0001-a.md)\n- [Second](docs/adr/0002-b.md)\n- [0010-c.md](docs/adr/0010-c.md)\n- [Z.md](docs/adr/Z.md)\n"
    );
    assert_eq!(
        loaded.snapshot,
        b"docs/adr/0001-a.md\x0011\x00First `one`\x00docs/adr/0002-b.md\x006\x00Second\x00docs/adr/0010-c.md\x009\x000010-c.md\x00docs/adr/Z.md\x004\x00Z.md\x00"
    );
    let loaded = s
        .load(
            "docs/README.md",
            "<!-- computed index src=adr title=filename -->",
        )
        .unwrap();
    assert_eq!(
        loaded.text,
        "- [0001-a.md](adr/0001-a.md)\n- [0002-b.md](adr/0002-b.md)\n- [0010-c.md](adr/0010-c.md)\n- [Z.md](adr/Z.md)\n- [notes.txt](adr/notes.txt)\n"
    );
    let loaded = s
        .load(
            "docs/README.md",
            "<!-- computed index src=adr/*.txt,adr/0001-a.md -->",
        )
        .unwrap();
    assert_eq!(
        loaded.text, "- [First `one`](adr/0001-a.md)\n- [notes.txt](adr/notes.txt)\n",
        "a file that is not Markdown is titled by its name"
    );
}

#[test]
fn the_template_is_left_out_and_an_empty_glob_is_a_hard_error() {
    let s = Sandbox::new();
    s.write("docs/a.md", "# A\n")
        .write("docs/index.md", "# Index\n");
    let loaded = s
        .load("docs/index.md", "<!-- computed index src=*.md -->")
        .unwrap();
    assert_eq!(loaded.text, "- [A](a.md)\n");
    let m = hard(s.snapshot("CLAUDE.md", "<!-- computed index src=docs/*.rs -->"));
    assert_eq!(m, "src=docs/*.rs matches nothing");
    let m = hard(s.snapshot("CLAUDE.md", "<!-- computed index src=../*.md -->"));
    assert!(m.contains("escapes"), "{m}");
}

#[test]
fn the_grammar_checks_src_and_title() {
    for (opener, needle) in [
        ("<!-- computed index -->", "index needs src="),
        (
            "<!-- computed index src=a.md#section=A -->",
            "not a projection",
        ),
        ("<!-- computed index src=a/*.md, -->", "entry is empty"),
        (
            "<!-- computed index src=a title=h2 -->",
            "expected h1 or filename",
        ),
        (
            "<!-- computed index src=a format=x -->",
            "unknown attribute",
        ),
    ] {
        let m = parse_error(opener);
        assert!(m.contains(needle), "{opener}: {m}");
    }
}

#[test]
fn a_body_edit_elsewhere_in_a_file_leaves_the_index_fresh() {
    let s = Sandbox::new();
    s.write("docs/a.md", "# A\n\nFirst draft.\n").write(
        "README.md",
        "# Docs\n\n<!-- computed index src=docs/*.md name=docs -->\n<!-- /computed -->\n",
    );
    assert_eq!(code(&s.run(&["run"])), Some(1));
    assert!(s.read("README.md").contains("\n- [A](docs/a.md)\n"));
    s.write("docs/a.md", "# A\n\nSecond draft.\n");
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    s.write("docs/a.md", "# A, renamed\n\nSecond draft.\n");
    let out = s.run(&["check"]);
    assert!(
        stderr(&out).contains("docs index stale"),
        "{}",
        stderr(&out)
    );
    s.write("docs/b.md", "# B\n");
    s.run(&["run"]);
    assert!(
        s.read("README.md")
            .contains("\n- [A, renamed](docs/a.md)\n- [B](docs/b.md)\n")
    );
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
}

/// The `adrs` region in this repository's `CLAUDE.md` runs
/// `scripts/adr-index.sh`; `index` must print what it prints.
#[test]
fn index_reproduces_the_adr_script() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = Command::new("/bin/sh")
        .arg("scripts/adr-index.sh")
        .current_dir(root)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(script.status.success());
    let mut p = Production::new(Ctx::for_template(&root.join("CLAUDE.md")));
    let loaded = p
        .load(&region("<!-- computed index src=\"docs/adr/*.md\" -->"))
        .unwrap();
    assert_eq!(loaded.text, String::from_utf8(script.stdout).unwrap());
}
