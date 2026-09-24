//! The `remote` loader and `computed update` end to end, against a server
//! on 127.0.0.1 that this test runs: no test reaches the internet.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use assert_cmd::prelude::*;
use sha2::{Digest, Sha256};

/// A server answering GET with whatever `pages` holds for the path, 404
/// otherwise, counting requests.
struct Server {
    base: String,
    pages: Arc<Mutex<HashMap<String, String>>>,
    hits: Arc<AtomicUsize>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let pages: Arc<Mutex<HashMap<String, String>>> = Arc::default();
        let hits: Arc<AtomicUsize> = Arc::default();
        let (p, h) = (pages.clone(), hits.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                h.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                        break;
                    }
                }
                let path = request.split(' ').nth(1).unwrap_or("").to_string();
                let page = p.lock().unwrap().get(&path).cloned();
                let (status, body) = match page {
                    Some(body) => ("200 OK", body),
                    None => ("404 Not Found", String::new()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Server { base, pages, hits }
    }

    fn put(&self, path: &str, body: &str) {
        self.pages
            .lock()
            .unwrap()
            .insert(path.to_string(), body.to_string());
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

fn digest(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn computed(root: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("computed")
        .unwrap()
        .current_dir(root)
        .env("XDG_CONFIG_HOME", root.join(".git/config-home"))
        .args(args)
        .output()
        .unwrap()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn update_pins_run_fetches_and_check_stays_offline() {
    let server = Server::start();
    server.put("/spec.md", "The spec, version one.\n");
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir(r.join(".git")).unwrap();
    let readme = r.join("README.md");
    let url = format!("{}/spec.md", server.base);
    fs::write(
        &readme,
        format!("# Spec\n\n<!-- computed remote url={url} name=spec -->\n<!-- /computed -->\n"),
    )
    .unwrap();

    // Unpinned: `check` reports it without a fetch, and `run` will not fetch.
    assert_eq!(computed(r, &["check"]).status.code(), Some(1));
    let out = computed(r, &["run"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("no sha256= pin; run `computed update`"),
        "{}",
        stderr(&out)
    );
    assert_eq!(server.hits(), 0);

    // `update --dry-run` shows the pin it would write.
    let one = digest("The spec, version one.\n");
    let before = fs::read_to_string(&readme).unwrap();
    let out = computed(r, &["update", "--dry-run"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stdout(&out).contains(&format!(
            "+<!-- computed remote url={url} sha256={one} name=spec -->"
        )),
        "{}",
        stdout(&out)
    );
    assert_eq!(fs::read_to_string(&readme).unwrap(), before);

    let out = computed(r, &["update"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        format!("README.md:3 spec remote stale written\n    now {one}\n")
    );
    assert!(
        fs::read_to_string(&readme)
            .unwrap()
            .contains(&format!("sha256={one} name=spec -->\n<!-- /computed -->"))
    );
    assert_eq!(computed(r, &["update"]).status.code(), Some(0));

    // Pinned: `run` fetches and renders; `check` answers without the network.
    let out = computed(r, &["run"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        fs::read_to_string(&readme)
            .unwrap()
            .contains("-->\n\nThe spec, version one.\n\n<!-- /computed in=")
    );
    let hits = server.hits();
    assert_eq!(computed(r, &["check"]).status.code(), Some(0));
    assert_eq!(server.hits(), hits);

    // Upstream moves: the region stays fresh, a forced run refuses the new
    // body, and `update` moves the pin so the next run renders it.
    server.put("/spec.md", "The spec, version two.\n");
    assert_eq!(computed(r, &["check"]).status.code(), Some(0));
    let out = computed(r, &["run", "--force"]);
    assert_eq!(out.status.code(), Some(1));
    let two = digest("The spec, version two.\n");
    assert!(
        stderr(&out).contains(&format!("pinned  {one}\n    fetched {two}")),
        "{}",
        stderr(&out)
    );
    assert!(fs::read_to_string(&readme).unwrap().contains("version one"));
    let out = computed(r, &["--format", "json", "update"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).contains("\"action\":\"written\""),
        "{}",
        stdout(&out)
    );
    assert_eq!(stderr(&out), "");
    assert_eq!(computed(r, &["check"]).status.code(), Some(1));
    assert_eq!(computed(r, &["run"]).status.code(), Some(1));
    assert!(fs::read_to_string(&readme).unwrap().contains("version two"));
    assert_eq!(computed(r, &["check"]).status.code(), Some(0));
}

#[test]
fn a_url_that_cannot_be_fetched_is_an_error_for_update() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir(r.join(".git")).unwrap();
    fs::write(
        r.join("README.md"),
        format!(
            "<!-- computed remote url={}/missing.md -->\n<!-- /computed -->\n",
            server.base
        ),
    )
    .unwrap();
    let out = computed(r, &["update"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out)
            .contains("README.md:1  remote error skipped; body kept\n    http://127.0.0.1:"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("404"), "{}", stderr(&out));
    let out = computed(r, &["update", "--only", "nope"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("no region is named \"nope\""));
}
