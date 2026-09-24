//! The `value` loader: one scalar of a TOML, JSON or YAML file.

mod sandbox;

use sandbox::{Sandbox, code, hard, parse_error, stderr};

const CARGO: &str = "[package]\nname = \"computed\"\nversion = \"0.2.0\"\nrust-version = \"1.88\"\n\n[dependencies]\nclap = \"4\"\n";

#[test]
fn the_text_is_the_scalar_and_the_snapshot_the_key_and_value() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO);
    let loaded = s
        .load(
            "README.md",
            "<!-- computed value src=Cargo.toml key=package.version -->",
        )
        .unwrap();
    assert_eq!(loaded.text, "0.2.0");
    assert_eq!(
        loaded.snapshot,
        b"Cargo.toml#key=package.version\x005\x000.2.0\x00"
    );
    s.write(
        "package.json",
        r#"{"engines": {"node": ">=20"}, "private": true, "workspaces": ["a", "b"]}"#,
    );
    for (key, want) in [
        ("engines.node", ">=20"),
        ("private", "true"),
        ("workspaces.1", "b"),
    ] {
        let opener = format!("<!-- computed value src=package.json key={key} -->");
        assert_eq!(s.load("README.md", &opener).unwrap().text, want);
    }
    s.write("ci.yml", "jobs:\n  test:\n    runs-on: ubuntu-24.04\n")
        .write("docs/README.md", "");
    let loaded = s
        .load(
            "docs/README.md",
            "<!-- computed value src=../ci.yml key=jobs.test.runs-on -->",
        )
        .unwrap();
    assert_eq!(loaded.text, "ubuntu-24.04");
}

#[test]
fn a_table_a_missing_key_or_a_second_line_is_a_hard_error() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO)
        .write("notes.toml", "text = \"\"\"\none\ntwo\"\"\"\n");
    for (opener, needle) in [
        (
            "<!-- computed value src=Cargo.toml key=package -->",
            "package is a table; value takes a string",
        ),
        (
            "<!-- computed value src=Cargo.toml key=package.license -->",
            "src=: Cargo.toml: key=package.license: package has no key \"license\"",
        ),
        (
            "<!-- computed value src=notes.toml key=text -->",
            "more than one line",
        ),
        ("<!-- computed value src=README.md key=a -->", "README.md"),
        ("<!-- computed value src=.. key=a -->", "escapes"),
    ] {
        let m = hard(s.snapshot("README.md", opener));
        assert!(m.contains(needle), "{opener}: {m}");
    }
}

#[test]
fn the_grammar_needs_src_and_a_key() {
    for (opener, needle) in [
        ("<!-- computed value key=a -->", "value needs src="),
        ("<!-- computed value src=a.toml -->", "value needs key="),
        ("<!-- computed value src=a.toml key=a..b -->", "dotted path"),
        (
            "<!-- computed value src=a.toml key=a lines=1 -->",
            "unknown attribute lines=",
        ),
    ] {
        let m = parse_error(opener);
        assert!(m.contains(needle), "{opener}: {m}");
    }
}

#[test]
fn another_key_changing_leaves_the_region_fresh() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO).write(
        "README.md",
        "Version <!-- x -->\n\n<!-- computed value src=Cargo.toml key=package.version name=version -->\n<!-- /computed -->\n",
    );
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(
        s.read("README.md")
            .contains("-->\n\n0.2.0\n\n<!-- /computed in="),
        "{}",
        s.read("README.md")
    );

    s.write("Cargo.toml", &CARGO.replace("clap = \"4\"", "clap = \"5\""));
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));

    s.write("Cargo.toml", &CARGO.replace("0.2.0", "0.3.0"));
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains("version value stale"),
        "{}",
        stderr(&out)
    );
    s.run(&["run"]);
    assert!(s.read("README.md").contains("\n0.3.0\n"));
}
