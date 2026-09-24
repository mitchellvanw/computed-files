//! `computed watch` end to end: a child process that renders on change and
//! stops on Ctrl-C.

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::prelude::*;

/// Polls `f` every 50 ms for up to ten seconds.
fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// The child, killed if the test fails before it is stopped.
struct Watching(Option<Child>);

impl Watching {
    /// Ctrl-C, then what the child printed and how it exited.
    fn interrupt(mut self) -> std::process::Output {
        let child = self.0.take().unwrap();
        // SAFETY: signalling our own child.
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
        child.wait_with_output().unwrap()
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
        }
    }
}

#[test]
fn watch_renders_on_change_and_stops_on_ctrl_c() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join(".git")).unwrap();
    fs::create_dir_all(r.join("src")).unwrap();
    fs::create_dir_all(r.join("target")).unwrap();
    fs::write(r.join(".gitignore"), "target/\n").unwrap();
    fs::write(r.join("src/main.rs"), "").unwrap();
    let notes = r.join("NOTES.md");
    fs::write(
        &notes,
        "# Notes\n\n<!-- computed tree src=src name=layout -->\n<!-- /computed -->\n",
    )
    .unwrap();

    let config = tempfile::tempdir().unwrap();
    let watching = Watching(Some(
        Command::cargo_bin("computed")
            .unwrap()
            .current_dir(r)
            .env("XDG_CONFIG_HOME", config.path())
            .arg("watch")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));

    eventually("the first pass", || read(&notes).contains("main.rs"));
    // Give the watcher a moment to settle past its own first write.
    std::thread::sleep(Duration::from_millis(500));

    fs::write(r.join("src/lib.rs"), "").unwrap();
    eventually("the new input", || read(&notes).contains("lib.rs"));

    // A change the .gitignore rules ignore runs nothing, and a new
    // template is picked up.
    fs::write(r.join("target/out"), "").unwrap();
    let other = r.join("docs.md");
    fs::write(
        &other,
        "<!-- computed tree src=src name=other -->\n<!-- /computed -->\n",
    )
    .unwrap();
    eventually("the new template", || read(&other).contains("lib.rs"));

    // Ctrl-C exits 0.
    let out = watching.interrupt();
    assert_eq!(out.status.code(), Some(0));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("NOTES.md:3 layout tree unrendered written"),
        "{err}"
    );
    assert!(
        err.contains("NOTES.md:3 layout tree stale written"),
        "{err}"
    );
    assert!(
        err.contains("docs.md:1 other tree unrendered written"),
        "{err}"
    );
    assert_eq!(
        err.matches("layout tree stale written").count(),
        1,
        "its own write does not run it again: {err}"
    );
}

mod sandbox;

#[test]
fn watch_takes_allow_for_remote_regions() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join(".git")).unwrap();
    let base = sandbox::serve("fetched\n");
    let notes = r.join("NOTES.md");
    fs::write(
        &notes,
        format!(
            "<!-- computed remote url={base}/doc.md sha256={} name=doc -->\n<!-- /computed -->\n",
            sandbox::pin("fetched\n")
        ),
    )
    .unwrap();
    let config = tempfile::tempdir().unwrap();
    let prefix = format!("{base}/");
    let watching = Watching(Some(
        Command::cargo_bin("computed")
            .unwrap()
            .current_dir(r)
            .env("XDG_CONFIG_HOME", config.path())
            .args(["watch", "--allow", &prefix])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    eventually("the allowed fetch", || read(&notes).contains("\nfetched\n"));
    assert_eq!(watching.interrupt().status.code(), Some(0));
}
