//! `computed guard` end to end: the Claude Code hooks' JSON in, the
//! decision out, and the plain file-against-proposed form.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use assert_cmd::prelude::*;

const TEMPLATE: &str =
    "# Notes\n\nProse.\n\n<!-- computed tree src=src name=layout -->\n<!-- /computed -->\n";

/// A repository with `NOTES.md` rendered, so its region is fresh.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join(".git")).unwrap();
    fs::create_dir_all(r.join("src")).unwrap();
    fs::write(r.join("src/main.rs"), "").unwrap();
    fs::write(r.join("NOTES.md"), TEMPLATE).unwrap();
    let out = computed(r, &["run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    dir
}

fn computed(dir: &Path, args: &[&str]) -> Command {
    let mut c = Command::cargo_bin("computed").unwrap();
    c.current_dir(dir).args(args);
    c
}

fn hook(dir: &Path, event: &str, input: &serde_json::Value) -> Output {
    let mut child = computed(dir, &["guard", "--hook", event])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn edit(dir: &Path, old: &str, new: &str) -> serde_json::Value {
    serde_json::json!({
        "session_id": "s",
        "hook_event_name": "PreToolUse",
        "cwd": dir.to_str().unwrap(),
        "tool_name": "Edit",
        "tool_input": {
            "file_path": dir.join("NOTES.md").to_str().unwrap(),
            "old_string": old,
            "new_string": new,
        }
    })
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn pre_denies_an_edit_to_a_body_and_names_the_region() {
    let dir = repo();
    let out = hook(
        dir.path(),
        "pre",
        &edit(dir.path(), "└── main.rs", "└── lib.rs"),
    );
    assert_eq!(out.status.code(), Some(0));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let specific = &doc["hookSpecificOutput"];
    assert_eq!(specific["hookEventName"], "PreToolUse");
    assert_eq!(specific["permissionDecision"], "deny");
    let reason = specific["permissionDecisionReason"].as_str().unwrap();
    assert!(
        reason.starts_with(
            "NOTES.md:5 `layout` is owned by computed (tree src=src name=layout); this edit changes its body."
        ),
        "{reason}"
    );
    assert!(reason.contains("computed run NOTES.md"), "{reason}");
}

#[test]
fn pre_allows_prose_and_openers() {
    let dir = repo();
    for (old, new) in [
        ("Prose.", "Better prose."),
        ("src=src name=layout", "src=src depth=1 name=layout"),
    ] {
        let out = hook(dir.path(), "pre", &edit(dir.path(), old, new));
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(stdout(&out), "", "{old} -> {new}");
    }
}

#[test]
fn pre_denies_a_write_that_breaks_a_closer() {
    let dir = repo();
    let current = fs::read_to_string(dir.path().join("NOTES.md")).unwrap();
    let broken = current.replace("<!-- /computed in=", "<!-- computed-ish in=");
    let input = serde_json::json!({
        "cwd": dir.path().to_str().unwrap(),
        "tool_name": "Write",
        "tool_input": {"file_path": "NOTES.md", "content": broken}
    });
    let out = hook(dir.path(), "pre", &input);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let reason = doc["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(
        reason.starts_with("NOTES.md:5: this edit breaks the computed markers"),
        "{reason}"
    );
}

#[test]
fn pre_ignores_files_without_markers_and_other_tools() {
    let dir = repo();
    fs::write(dir.path().join("plain.md"), "text\n").unwrap();
    let input = serde_json::json!({
        "cwd": dir.path().to_str().unwrap(),
        "tool_name": "Edit",
        "tool_input": {"file_path": "plain.md", "old_string": "text", "new_string": "t"}
    });
    assert_eq!(stdout(&hook(dir.path(), "pre", &input)), "");
    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {"command": "true"}
    });
    assert_eq!(stdout(&hook(dir.path(), "pre", &input)), "");
}

#[test]
fn post_reports_drift_as_context_and_is_quiet_when_fresh() {
    let dir = repo();
    let post = serde_json::json!({
        "cwd": dir.path().to_str().unwrap(),
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "tool_input": {"file_path": "NOTES.md", "old_string": "x", "new_string": "y"},
        "tool_response": {}
    });
    let out = hook(dir.path(), "post", &post);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "", "a fresh file adds nothing");

    // The opener edit the pre hook allowed has landed: the region is stale.
    let path = dir.path().join("NOTES.md");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("src=src name", "src=src depth=1 name")).unwrap();
    let out = hook(dir.path(), "post", &post);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    let context = doc["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        context.contains("NOTES.md:5 layout tree stale"),
        "{context}"
    );
    assert!(context.contains("computed run NOTES.md"), "{context}");
}

#[test]
fn the_plain_form_names_what_the_proposed_text_changes() {
    let dir = repo();
    let current = fs::read_to_string(dir.path().join("NOTES.md")).unwrap();
    fs::write(
        dir.path().join("proposed"),
        current.replace("└── main.rs", "└── x"),
    )
    .unwrap();
    let out = computed(dir.path(), &["guard", "NOTES.md", "--proposed", "proposed"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "NOTES.md:5 layout tree body changed\n"
    );

    let out = computed(
        dir.path(),
        &[
            "--format",
            "json",
            "guard",
            "NOTES.md",
            "--proposed",
            "proposed",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["regions"][0]["changed"], "body");
    assert_eq!(doc["regions"][0]["name"], "layout");

    fs::write(dir.path().join("proposed"), format!("Intro.\n{current}")).unwrap();
    let out = computed(dir.path(), &["guard", "NOTES.md", "--proposed", "proposed"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn the_plain_form_needs_both_paths() {
    let dir = repo();
    let out = computed(dir.path(), &["guard", "NOTES.md"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = computed(dir.path(), &["guard", "--hook", "pre", "NOTES.md"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
