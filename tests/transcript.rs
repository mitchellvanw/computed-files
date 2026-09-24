//! The `transcript` loader end to end: it needs trust as exec does, and
//! `workdir=copy` lets a step change files without touching the repository.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use assert_cmd::prelude::*;

fn computed(root: &Path, args: &[&str]) -> std::process::Output {
    // The steps run `computed` too: the one under test comes first on PATH.
    let bin = cargo_bin("computed");
    let path = format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::cargo_bin("computed")
        .unwrap()
        .current_dir(root)
        .env("XDG_CONFIG_HOME", root.join(".git/config-home"))
        .env("PATH", path)
        .args(args)
        .output()
        .unwrap()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_transcript_shows_a_session_run_in_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join(".git")).unwrap();
    fs::create_dir_all(r.join("src")).unwrap();
    fs::write(r.join("src/main.rs"), "").unwrap();
    fs::write(
        r.join("LAYOUT.md"),
        "<!-- computed tree src=src -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let readme = r.join("README.md");
    fs::write(
        &readme,
        "# Drift\n\n<!-- computed transcript steps=\"touch src/watcher.rs ;; computed check LAYOUT.md ;; echo $?\" inputs=src,LAYOUT.md workdir=copy -->\n<!-- /computed -->\n",
    )
    .unwrap();

    // Untrusted: skipped as an exec region is.
    let out = computed(r, &["run"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("README.md:3  transcript untrusted skipped; run `computed trust`"),
        "{}",
        stderr(&out)
    );

    let out = computed(r, &["run", "--trust"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let text = fs::read_to_string(&readme).unwrap();
    assert!(
        text.contains(
            "workdir=copy | do not edit; run computed -->\n```console\n$ touch src/watcher.rs\n$ computed check LAYOUT.md\nLAYOUT.md:1  tree stale\n$ echo $?\n1\n```\n"
        ),
        "{text}"
    );
    assert!(
        !r.join("src/watcher.rs").exists(),
        "the copy took the touch"
    );
    let out = computed(r, &["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    // An input changes: the transcript is stale.
    fs::write(r.join("src/lib.rs"), "").unwrap();
    let out = computed(r, &["check", "README.md"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("transcript stale"),
        "{}",
        stderr(&out)
    );
}
