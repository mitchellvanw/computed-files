//! Regions inside a line: one value in a sentence, kept current like any
//! region, reported at its line and column.

mod sandbox;

use sandbox::{Sandbox, code, stderr};

const CARGO: &str = "[package]\nname = \"demo\"\nversion = \"0.2.0\"\n";

const README: &str = "# Demo\n\nThe current release is <!-- computed value src=Cargo.toml key=package.version name=version -->?<!-- /computed -->, see `<!-- computed tree -->`.\n";

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_value_in_a_sentence_renders_checks_and_goes_stale() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO).write("README.md", README);
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "README.md:3:24 version value unrendered written\n"
    );
    let text = s.read("README.md");
    let (head, tail) = text.split_once("<!-- /computed in=").unwrap();
    assert_eq!(
        head,
        "# Demo\n\nThe current release is <!-- computed value src=Cargo.toml key=package.version name=version -->0.2.0"
    );
    assert!(
        tail.ends_with(" -->, see `<!-- computed tree -->`.\n"),
        "{tail}"
    );
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));

    s.write("Cargo.toml", &CARGO.replace("0.2.0", "0.3.0"));
    let out = s.run(&["--format", "json", "check", "--only", "version"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stdout(&out).contains(
            "\"regions\":[{\"line\":3,\"column\":24,\"name\":\"version\",\"loader\":\"value\",\"state\":\"stale\""
        ),
        "{}",
        stdout(&out)
    );
    let out = s.run(&["run", "--dry-run"]);
    assert!(stdout(&out).contains("+The current release is <!-- computed value src=Cargo.toml key=package.version name=version -->0.3.0<!-- /computed in="), "{}", stdout(&out));
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(s.read("README.md").contains("-->0.3.0<!--"));
}

#[test]
fn a_hand_edit_is_refused_and_clean_empties_the_body() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO).write("README.md", README);
    s.run(&["run"]);
    let text = s.read("README.md");
    s.write("README.md", &text.replace("-->0.2.0<!--", "-->9.9.9<!--"));
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains("README.md:3:24 version value edited refused"),
        "{}",
        stderr(&out)
    );
    let out = s.run(&["clean", "--force"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert_eq!(s.read("README.md"), README.replace("-->?<!--", "--><!--"));
}

#[test]
fn text_of_more_than_one_line_fails_and_keeps_the_body() {
    let s = Sandbox::new();
    s.write("two.txt", "a\nb\n").write(
        "README.md",
        "x <!-- computed file src=two.txt -->kept<!-- /computed --> y\n",
    );
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains("the text is 2 lines, and a region inside a line holds one"),
        "{}",
        stderr(&out)
    );
    assert!(s.read("README.md").contains("-->kept<!--"));
}

#[test]
fn an_inline_and_a_block_region_never_share_an_input_sum() {
    let s = Sandbox::new();
    s.write("v.txt", "1\n").write(
        "README.md",
        "a <!-- computed file src=v.txt -->x<!-- /computed --> b\n\n<!-- computed file src=v.txt -->\n<!-- /computed -->\n",
    );
    s.run(&["run"]);
    let text = s.read("README.md");
    let sums: Vec<&str> = text
        .match_indices("in=")
        .map(|(i, _)| &text[i + 3..i + 67])
        .collect();
    assert_eq!(sums.len(), 2);
    assert_ne!(sums[0], sums[1]);
}

#[test]
fn a_toc_of_a_heading_that_holds_a_value_settles_in_one_run() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO).write(
        "README.md",
        "<!-- computed toc -->\n<!-- /computed -->\n\n## Release <!-- computed value src=Cargo.toml key=package.version -->?<!-- /computed -->\n",
    );
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(
        s.read("README.md")
            .contains("\n- [Release 0.2.0](#release-020)\n"),
        "{}",
        s.read("README.md")
    );
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    s.write("Cargo.toml", &CARGO.replace("0.2.0", "1.0.0"));
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(
        s.read("README.md")
            .contains("\n- [Release 1.0.0](#release-100)\n")
    );
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
}

#[test]
fn a_file_region_reads_a_template_without_its_inline_sums() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO)
        .write("docs/part.md", "Version <!-- computed value src=../Cargo.toml key=package.version -->?<!-- /computed -->.\n")
        .write(
            "README.md",
            "<!-- computed file src=docs/part.md -->\n<!-- /computed -->\n",
        );
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(
        s.read("README.md")
            .contains("\nVersion <!-- computed value src=../Cargo.toml key=package.version -->0.2.0<!-- /computed -->.\n"),
        "{}",
        s.read("README.md")
    );
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
}

#[test]
fn an_html_page_holds_a_value_inside_its_pre() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO).write(
        "site/index.html",
        "<pre><code>cargo install demo # v<!-- computed value src=../Cargo.toml key=package.version -->?<!-- /computed --></code></pre>\n<script>\nconst m = '<!-- computed tree -->';\n</script>\n",
    );
    let out = s.run(&["run", "site/index.html"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(s.read("site/index.html").contains(
        "# v<!-- computed value src=../Cargo.toml key=package.version -->0.2.0<!-- /computed in="
    ));
    let out = s.run(&["check", "site/index.html"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
}

#[test]
fn guard_refuses_an_edit_to_the_value_and_allows_the_sentence() {
    let s = Sandbox::new();
    s.write("Cargo.toml", CARGO).write("README.md", README);
    s.run(&["run"]);
    let text = s.read("README.md");
    s.write("edited.md", &text.replace("-->0.2.0<!--", "-->9.9.9<!--"))
        .write(
            "prose.md",
            &text.replace("The current release", "Our latest release"),
        );
    let out = s.run(&["guard", "README.md", "--proposed", "edited.md"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains("README.md:3:24 version value body changed"),
        "{}",
        stderr(&out)
    );
    let out = s.run(&["guard", "README.md", "--proposed", "prose.md"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
}

#[test]
fn stats_and_adopt_know_a_region_inside_a_line() {
    let s = Sandbox::new();
    s.write("v.txt", "1\n").write(
        "README.md",
        "a <!-- computed file src=v.txt name=v -->x<!-- /computed --> b\n",
    );
    s.run(&["run"]);
    let out = s.run(&["stats"]);
    assert!(
        stdout(&out).starts_with("README.md:1:3 v file 0 lines 1 byte ~1 token\n"),
        "{}",
        stdout(&out)
    );
    let text = s.read("README.md");
    s.write("README.md", &text.replace("-->1<!--", "-->2<!--"));
    let out = s.run(&["adopt", "README.md"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains(
            "README.md:1:3 v file edited refused; a region inside a line is not written back"
        ),
        "{}",
        stderr(&out)
    );
}
