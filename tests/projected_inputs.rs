//! Exec `inputs=` entries with a projection: `path#lines=`, `#section=`,
//! `#anchor=` or `#key=` on a literal path.

mod sandbox;

use sandbox::{Sandbox, code, hard, parse_error, stderr};

const CARGO: &str =
    "[package]\nname = \"computed\"\nversion = \"0.2.0\"\n\n[dependencies]\nclap = \"4\"\n";

fn repo() -> Sandbox {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO)
        .write("src/cli.rs", "one\ntwo\nthree\nfour\n")
        .write("docs/a.md", "# A\n")
        .write("notes/#1.md", "hash\n");
    s
}

#[test]
fn a_projected_entry_snapshots_its_slice_in_path_order() {
    let s = repo();
    let snap = s
        .snapshot(
            "CLAUDE.md",
            "<!-- computed exec cmd=true inputs=\"src/cli.rs#lines=2-3,docs/*.md,Cargo.toml#key=package.version\" -->",
        )
        .unwrap();
    assert_eq!(
        snap,
        b"Cargo.toml#key=package.version\x007\x00\"0.2.0\"\x00docs/a.md\x004\x00# A\n\x00src/cli.rs#lines=2-3\x0010\x00two\nthree\n\x00"
    );
    let snap = s
        .snapshot(
            "CLAUDE.md",
            "<!-- computed exec cmd=true inputs=./Cargo.toml#key=dependencies,Cargo.toml -->",
        )
        .unwrap();
    assert_eq!(
        String::from_utf8(snap).unwrap(),
        format!(
            "Cargo.toml\0{}\0{CARGO}\0Cargo.toml#key=dependencies\x0012\0{{\"clap\":\"4\"}}\0",
            CARGO.len()
        ),
        "a file taken whole and in part is two entries"
    );
}

#[test]
fn a_hash_that_starts_no_projection_is_part_of_the_path() {
    let s = repo();
    let snap = s
        .snapshot(
            "CLAUDE.md",
            "<!-- computed exec cmd=true inputs=notes/#1.md -->",
        )
        .unwrap();
    assert_eq!(snap, b"notes/#1.md\x005\x00hash\n\x00");
}

#[test]
fn a_projection_that_finds_nothing_is_a_hard_error() {
    let s = repo();
    for (inputs, needle) in [
        (
            "Cargo.toml#key=package.license",
            "inputs=Cargo.toml#key=package.license: package has no key \"license\"",
        ),
        (
            "src/cli.rs#lines=3-9",
            "inputs=src/cli.rs#lines=3-9: the file has 4 lines",
        ),
        (
            "missing.toml#key=a",
            "inputs=missing.toml#key=a matches nothing",
        ),
        ("src#lines=1", "src is not a file"),
        ("CLAUDE.md#section=A", "is this file"),
        ("../x.md#lines=1", ""),
    ] {
        s.write("CLAUDE.md", "");
        let opener = format!("<!-- computed exec cmd=true inputs=\"{inputs}\" -->");
        let m = hard(s.snapshot("CLAUDE.md", &opener));
        assert!(m.contains(needle), "{inputs}: {m}");
    }
}

#[test]
fn the_grammar_checks_every_projection() {
    for (opener, needle) in [
        (
            "<!-- computed exec cmd=x inputs=src/*.rs#lines=1-2 -->",
            "inputs=src/*.rs#lines=1-2: a projection needs a literal path",
        ),
        (
            "<!-- computed exec cmd=x inputs=a.md#lines=2-1 -->",
            "inputs=a.md#lines=2-1: the range ends before it starts",
        ),
        (
            "<!-- computed exec cmd=x inputs=docs/*.md,a.toml#key=a..b -->",
            "inputs=a.toml#key=a..b: expected a dotted path",
        ),
        (
            "<!-- computed exec cmd=x inputs=#section=A -->",
            "needs a path before the #",
        ),
    ] {
        let m = parse_error(opener);
        assert!(m.contains(needle), "{opener}: {m}");
    }
}

#[test]
fn an_edit_outside_the_projection_leaves_the_region_fresh() {
    let s = repo();
    s.write(
        "README.md",
        "<!-- computed exec cmd=\"grep ^version Cargo.toml\" inputs=Cargo.toml#key=package.version name=v -->\n<!-- /computed -->\n",
    );
    let out = s.run(&["run", "--trust"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(s.read("README.md").contains("\nversion = \"0.2.0\"\n"));
    s.write("Cargo.toml", &CARGO.replace("clap = \"4\"", "clap = \"5\""));
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    s.write("Cargo.toml", &CARGO.replace("0.2.0", "0.3.0"));
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(1));
    assert!(stderr(&out).contains("v exec stale"), "{}", stderr(&out));
}
