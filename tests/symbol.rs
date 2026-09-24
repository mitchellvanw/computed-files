//! The `symbol` loader end to end: the item renders, edits elsewhere in the
//! file leave it fresh, an edit to the item makes it stale.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;

const LIB: &str = "//! The crate.\n\n/// Adds two numbers.\n///\n/// Wraps on overflow.\n#[inline]\npub fn add(a: u8, b: u8) -> u8 {\n    a.wrapping_add(b)\n}\n\npub struct Counter(u8);\n\nimpl Counter {\n    /// Counts one more.\n    pub fn bump(&mut self) {\n        self.0 += 1;\n    }\n}\n";

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join(".git")).unwrap();
    fs::create_dir_all(r.join("src")).unwrap();
    fs::write(r.join("src/lib.rs"), LIB).unwrap();
    dir
}

fn computed(root: &Path, args: &[&str]) -> std::process::Output {
    let config = root.join(".config");
    Command::cargo_bin("computed")
        .unwrap()
        .current_dir(root)
        .env("XDG_CONFIG_HOME", config)
        .args(args)
        .output()
        .unwrap()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_symbol_renders_and_stays_fresh_through_edits_elsewhere() {
    let dir = repo();
    let r = dir.path();
    fs::write(
        r.join("README.md"),
        "# Api\n\n<!-- computed symbol src=src/lib.rs item=add part=signature -->\n<!-- /computed -->\n\n<!-- computed symbol src=src/lib.rs item=add part=doc -->\n<!-- /computed -->\n\n<!-- computed symbol src=src/lib.rs item=Counter.bump -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = computed(r, &["run"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let readme = fs::read_to_string(r.join("README.md")).unwrap();
    assert!(
        readme.contains(
            "part=signature | do not edit; run computed -->\n```rust\npub fn add(a: u8, b: u8) -> u8\n```\n"
        ),
        "{readme}"
    );
    assert!(
        readme.contains(
            "part=doc | do not edit; run computed -->\n\nAdds two numbers.\n\nWraps on overflow.\n\n<!--"
        ),
        "a doc comment is Markdown, so part=doc defaults to raw: {readme}"
    );
    assert!(
        readme.contains(
            "```rust\n/// Counts one more.\npub fn bump(&mut self) {\n    self.0 += 1;\n}\n```"
        ),
        "{readme}"
    );
    assert_eq!(computed(r, &["check"]).status.code(), Some(0));

    // An edit outside every item the regions show leaves them fresh.
    let edited = LIB.replace("//! The crate.", "//! The crate, renamed.") + "\npub fn later() {}\n";
    fs::write(r.join("src/lib.rs"), &edited).unwrap();
    let out = computed(r, &["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    // The function body is not part of its signature; the doc is its own part.
    fs::write(
        r.join("src/lib.rs"),
        edited.replace("a.wrapping_add(b)", "a.saturating_add(b)"),
    )
    .unwrap();
    let out = computed(r, &["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    fs::write(
        r.join("src/lib.rs"),
        edited.replace("self.0 += 1;", "self.0 += 2;"),
    )
    .unwrap();
    let out = computed(r, &["check"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert_eq!(
        err, "README.md:17  symbol stale\n",
        "only the method's region"
    );
}

#[test]
fn a_symbol_that_cannot_be_found_is_a_region_error() {
    let dir = repo();
    let r = dir.path();
    fs::write(r.join("src/notes.txt"), "text\n").unwrap();
    fs::write(
        r.join("README.md"),
        "<!-- computed symbol src=src/lib.rs item=missing -->\n<!-- /computed -->\n\n<!-- computed symbol src=src/notes.txt item=x -->\n<!-- /computed -->\n\n<!-- computed symbol src=src/lib.rs item=add lang=text as=raw -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = computed(r, &["run"]);
    assert_eq!(out.status.code(), Some(2));
    let err = stderr(&out);
    assert!(
        err.contains("item=missing: no such item in src/lib.rs"),
        "{err}"
    );
    assert!(err.contains("no grammar for this extension"), "{err}");
    let readme = fs::read_to_string(r.join("README.md")).unwrap();
    assert!(
        readme.contains("as=raw | do not edit; run computed -->\n\n/// Adds two numbers."),
        "the other region still renders, with the sink it names: {readme}"
    );
}
