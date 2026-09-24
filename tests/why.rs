//! `why` end to end, against real git repositories: the commit a region
//! was rendered from, recomputed, and what changed since.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use assert_cmd::prelude::*;

const CLAUDE: &str = "# Notes\n\n<!-- computed tree src=src name=layout -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"cat docs/*.md\" inputs=docs/*.md name=docs -->\n<!-- /computed -->\n\n<!-- computed file src=README.md name=readme -->\n<!-- /computed -->\n\n<!-- computed exec cmd=date volatile name=clock -->\n<!-- /computed -->\n";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn computed(dir: &Path, args: &[&str]) -> Output {
    Command::cargo_bin("computed")
        .unwrap()
        .current_dir(dir)
        .env("XDG_CONFIG_HOME", dir.join(".git/config-home"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", message]);
    git(dir, &["rev-parse", "--short", "HEAD"])
        .trim()
        .to_string()
}

/// A repository whose regions were rendered and committed, then whose
/// inputs changed in a second commit that did not render them.
fn repo() -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    git(d, &["init", "-q", "-b", "main"]);
    git(d, &["config", "user.name", "Test"]);
    git(d, &["config", "user.email", "test@example.com"]);
    fs::create_dir_all(d.join("src")).unwrap();
    fs::create_dir_all(d.join("docs")).unwrap();
    fs::write(d.join("src/main.rs"), "").unwrap();
    fs::write(d.join("src/old.rs"), "").unwrap();
    fs::write(d.join("docs/a.md"), "# A\n").unwrap();
    fs::write(d.join("docs/gone.md"), "# Gone\n").unwrap();
    fs::write(d.join("README.md"), "Readme\n").unwrap();
    fs::write(d.join("CLAUDE.md"), CLAUDE).unwrap();
    assert_eq!(computed(d, &["run", "--trust"]).status.code(), Some(1));
    let rendered = commit(d, "Render the notes");
    fs::write(d.join("src/lib.rs"), "").unwrap();
    fs::remove_file(d.join("src/old.rs")).unwrap();
    fs::write(d.join("docs/a.md"), "# A, renamed\n").unwrap();
    fs::write(d.join("docs/b.md"), "# B\n").unwrap();
    fs::remove_file(d.join("docs/gone.md")).unwrap();
    let changed = commit(d, "Change the inputs");
    (dir, rendered, changed)
}

#[test]
fn a_stale_tree_names_the_render_commit_the_paths_and_the_commits_since() {
    let (dir, rendered, changed) = repo();
    let out = computed(dir.path(), &["why", "CLAUDE.md", "--only", "layout"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.starts_with("CLAUDE.md:3 layout tree stale\n"),
        "{text}"
    );
    assert!(
        text.contains(&format!("rendered from {rendered} ")) && text.contains("Render the notes"),
        "{text}"
    );
    assert!(text.contains("    + lib.rs\n"), "{text}");
    assert!(text.contains("    - old.rs\n"), "{text}");
    assert!(
        !text.contains("main.rs"),
        "unchanged paths are not listed: {text}"
    );
    assert!(
        text.contains(&format!("{changed} Change the inputs")),
        "{text}"
    );
}

#[test]
fn a_stale_exec_region_names_files_added_removed_and_changed() {
    let (dir, _, _) = repo();
    let out = computed(dir.path(), &["why", "CLAUDE.md", "--line", "11"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.starts_with("CLAUDE.md:11 docs exec stale\n"), "{text}");
    assert!(text.contains("    added   docs/b.md\n"), "{text}");
    assert!(text.contains("    removed docs/gone.md\n"), "{text}");
    assert!(text.contains("    changed docs/a.md\n"), "{text}");
    assert!(
        !text.contains("+# A, renamed"),
        "content diffs need -v: {text}"
    );

    let out = computed(dir.path(), &["-v", "why", "CLAUDE.md", "--only", "docs"]);
    let text = stdout(&out);
    assert!(text.contains("        -# A\n"), "{text}");
    assert!(text.contains("        +# A, renamed\n"), "{text}");
}

#[test]
fn every_region_is_answered_when_none_is_named() {
    let (dir, _, _) = repo();
    let out = computed(dir.path(), &["why", "CLAUDE.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("CLAUDE.md:3 layout tree stale"), "{text}");
    assert!(text.contains("CLAUDE.md:11 docs exec stale"), "{text}");
    assert!(text.contains("CLAUDE.md:18 readme file fresh\n"), "{text}");
    assert!(
        text.contains("CLAUDE.md:24 clock exec volatile\n"),
        "{text}"
    );
}

#[test]
fn a_changed_opener_shows_both_forms_and_a_working_tree_change_says_so() {
    let (dir, _, _) = repo();
    let d = dir.path();
    let text = fs::read_to_string(d.join("CLAUDE.md")).unwrap();
    fs::write(
        d.join("CLAUDE.md"),
        text.replace(
            "file src=README.md name=readme",
            "file src=README.md name=readme as=fence",
        ),
    )
    .unwrap();
    fs::write(d.join("README.md"), "Readme, edited\n").unwrap();
    let out = computed(d, &["why", "CLAUDE.md", "--only", "readme"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("    opener changed\n"), "{text}");
    assert!(
        text.contains("        - <!-- computed file src=README.md name=readme -->\n"),
        "{text}"
    );
    assert!(
        text.contains("        + <!-- computed file src=README.md name=readme as=fence -->\n"),
        "{text}"
    );
    assert!(text.contains("    changed README.md\n"), "{text}");
    assert!(text.contains("not committed"), "{text}");
}

#[test]
fn a_render_no_commit_holds_is_exit_2() {
    let (dir, _, _) = repo();
    let d = dir.path();
    assert_eq!(computed(d, &["run", "--trust"]).status.code(), Some(1));
    fs::write(d.join("src/new.rs"), "").unwrap();
    let out = computed(d, &["why", "CLAUDE.md", "--only", "layout"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stdout(&out).contains("no commit"), "{}", stdout(&out));
}

#[test]
fn the_latest_commit_whose_tree_reproduces_the_sum_is_the_one_explained_from() {
    let (dir, rendered, changed) = repo();
    let d = dir.path();
    // A prose-only commit keeps the region's sums, but its tree already has
    // the changed inputs: it does not reproduce them, the render commit does.
    let text = fs::read_to_string(d.join("CLAUDE.md")).unwrap();
    fs::write(
        d.join("CLAUDE.md"),
        text.replace("# Notes", "# Notes, retitled"),
    )
    .unwrap();
    commit(d, "Retitle");
    let out = computed(d, &["why", "CLAUDE.md", "--only", "layout"]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(
        text.contains(&format!("rendered from {rendered} ")),
        "{text}"
    );
    assert!(
        text.contains(&format!("{changed} Change the inputs")),
        "{text}"
    );
    assert!(!text.contains("Retitle"), "{text}");
}

#[test]
fn a_commit_that_does_not_reproduce_its_sum_is_exit_2_with_what_is_known() {
    let (dir, _, _) = repo();
    let d = dir.path();
    // Rendered with a file present that was never committed.
    fs::write(d.join("src/untracked.rs"), "").unwrap();
    assert_eq!(computed(d, &["run", "--trust"]).status.code(), Some(1));
    git(d, &["add", "CLAUDE.md"]);
    git(d, &["commit", "-qm", "Render with an untracked file"]);
    fs::remove_file(d.join("src/untracked.rs")).unwrap();
    let out = computed(d, &["why", "CLAUDE.md", "--only", "layout"]);
    assert_eq!(out.status.code(), Some(2), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("did not reproduce"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn fresh_unrendered_and_outside_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::create_dir_all(d.join("src")).unwrap();
    fs::write(
        d.join("notes.md"),
        "<!-- computed tree src=src name=layout -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let out = computed(d, &["why", "notes.md"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).starts_with("notes.md:1 layout tree unrendered\n"));
    assert_eq!(computed(d, &["run"]).status.code(), Some(1));
    let out = computed(d, &["why", "notes.md"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "notes.md:1 layout tree fresh\n");
    fs::write(d.join("src/a.rs"), "").unwrap();
    let out = computed(d, &["why", "notes.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stdout(&out).contains("not in a git repository"),
        "{}",
        stdout(&out)
    );
    let out = computed(d, &["why", "notes.md", "--only", "nope"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("no region is named \"nope\""));
}

#[test]
fn a_hand_edit_is_named_and_shown_with_v() {
    let (dir, rendered, _) = repo();
    let d = dir.path();
    let text = fs::read_to_string(d.join("CLAUDE.md")).unwrap();
    fs::write(
        d.join("CLAUDE.md"),
        text.replace("\nReadme\n", "\nReadme, by hand\n"),
    )
    .unwrap();
    let out = computed(d, &["why", "CLAUDE.md", "--only", "readme"]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert_eq!(
        stdout(&out),
        "CLAUDE.md:18 readme file edited\n    the body was changed by hand after it was rendered\n"
    );
    let out = computed(d, &["-v", "why", "CLAUDE.md", "--only", "readme"]);
    let text = stdout(&out);
    assert!(text.contains(&format!("rendered in {rendered} ")), "{text}");
    assert!(text.contains("        +Readme, by hand\n"), "{text}");
}

#[test]
fn a_use_region_is_explained_through_the_recipe_of_its_commit() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    git(d, &["init", "-q", "-b", "main"]);
    git(d, &["config", "user.name", "Test"]);
    git(d, &["config", "user.email", "test@example.com"]);
    fs::create_dir_all(d.join("src")).unwrap();
    fs::write(d.join("src/main.rs"), "").unwrap();
    fs::write(
        d.join("computed.toml"),
        "[recipe.layout]\nloader = \"tree\"\nsrc = \"src\"\n",
    )
    .unwrap();
    fs::write(
        d.join("NOTES.md"),
        "<!-- computed use recipe=layout name=layout -->\n<!-- /computed -->\n",
    )
    .unwrap();
    assert_eq!(computed(d, &["run"]).status.code(), Some(1));
    let rendered = commit(d, "Render");
    // An input changes: the expansion is the same, the listing is not.
    fs::write(d.join("src/lib.rs"), "").unwrap();
    commit(d, "Add lib");
    let out = computed(d, &["why", "NOTES.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.starts_with("NOTES.md:1 layout tree stale\n"), "{text}");
    assert!(
        text.contains(&format!("rendered from {rendered} ")),
        "{text}"
    );
    assert!(text.contains("    + lib.rs\n"), "{text}");
    assert!(!text.contains("opener changed"), "{text}");
    // The recipe changes: the expanded openers differ.
    fs::write(
        d.join("computed.toml"),
        "[recipe.layout]\nloader = \"tree\"\nsrc = \"src\"\ndepth = 1\n",
    )
    .unwrap();
    commit(d, "Shallower");
    let text = stdout(&computed(d, &["why", "NOTES.md"]));
    assert!(text.contains("opener changed"), "{text}");
    assert!(
        text.contains("+ <!-- computed tree depth=1 src=src name=layout -->"),
        "{text}"
    );
}
