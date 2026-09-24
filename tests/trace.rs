//! `computed trace` end to end with this platform's real tracer: strace on
//! Linux, sandbox-exec and the unified log on macOS. Skipped, with a note,
//! where neither can trace.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;

const TEMPLATE: &str = "# Doc\n\n<!-- computed exec cmd=\"cat docs/adr/*.md; grep version Cargo.toml\" inputs=docs/adr/*.md,src/main.rs name=adrs -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"cat docs/adr/0001.md\" inputs=docs/adr/0001.md name=ok -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"cat src/main.rs\" volatile name=vol -->\n<!-- /computed -->\n";

struct Repo {
    dir: tempfile::TempDir,
    config: tempfile::TempDir,
}

impl Repo {
    fn new() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        for d in [".git", "docs/adr", "src"] {
            fs::create_dir_all(r.join(d)).unwrap();
        }
        fs::write(r.join("docs/adr/0001.md"), "# One\n").unwrap();
        fs::write(r.join("docs/adr/0002.md"), "# Two\n").unwrap();
        fs::write(r.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(r.join("Cargo.toml"), "version = \"1\"\n").unwrap();
        fs::write(r.join("DOC.md"), TEMPLATE).unwrap();
        Repo {
            dir,
            config: tempfile::tempdir().unwrap(),
        }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::cargo_bin("computed").unwrap();
        c.current_dir(self.path())
            .env("XDG_CONFIG_HOME", self.config.path())
            .args(args);
        c
    }
    fn doc(&self) -> String {
        fs::read_to_string(self.path().join("DOC.md")).unwrap()
    }
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Whether this machine can trace; says why not when it cannot.
fn tracer() -> bool {
    match computed::trace::detect() {
        Ok(_) => true,
        Err(why) => {
            eprintln!("skipped: {why}");
            false
        }
    }
}

#[test]
fn undeclared_reads_and_unused_inputs_are_reported_and_written() {
    if !tracer() {
        return;
    }
    let repo = Repo::new();
    let out = repo.cmd(&["trace", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("DOC.md:3 adrs exec undeclared+unused\n    undeclared: Cargo.toml\n    unused: src/main.rs\n    suggest: inputs=Cargo.toml,docs/adr/*.md\n"),
        "{err}"
    );
    assert!(!err.contains(" ok "), "a complete region is silent: {err}");
    assert!(!err.contains(" vol "), "a volatile region is silent: {err}");
    assert!(!repo.doc().contains("# One"), "trace writes no body");

    let out = repo.cmd(&["-v", "trace", "--trust"]).output().unwrap();
    let err = stderr(&out);
    assert!(err.contains("DOC.md:6 ok   exec complete"), "{err}");
    assert!(
        err.contains("DOC.md:9 vol  exec volatile\n    read: src/main.rs\n"),
        "{err}"
    );

    let out = repo
        .cmd(&["trace", "--trust", "--write", "--only", "adrs"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("rewritten; the region is stale until `computed run`"),
        "{}",
        stderr(&out)
    );
    assert!(
        repo.doc()
            .contains("inputs=Cargo.toml,docs/adr/*.md name=adrs -->"),
        "{}",
        repo.doc()
    );
    let out = repo.cmd(&["trace", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn a_rewritten_opener_makes_a_rendered_region_stale() {
    if !tracer() {
        return;
    }
    let repo = Repo::new();
    assert_eq!(
        repo.cmd(&["run", "--trust"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        repo.cmd(&["check"]).output().unwrap().status.code(),
        Some(0)
    );
    repo.cmd(&["trace", "--trust", "--write"]).output().unwrap();
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("DOC.md:3 adrs exec stale"),
        "{}",
        stderr(&out)
    );
    assert!(repo.doc().contains("| do not edit; run computed -->"));
}

#[test]
fn trace_needs_trust_and_speaks_json() {
    if !tracer() {
        return;
    }
    let repo = Repo::new();
    let out = repo.cmd(&["trace"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("DOC.md:3 adrs exec untrusted"),
        "{}",
        stderr(&out)
    );
    let out = repo
        .cmd(&["--format", "json", "trace", "--trust", "--only", "ok"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr(&out), "");
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"exit\":0,\"files\":[{\"path\":\"DOC.md\",\"error\":null,\"regions\":[{\"line\":6,\"column\":null,\"name\":\"ok\",\"loader\":\"exec\",\"verdict\":\"complete\",\"reads\":[\"docs/adr/0001.md\"],\"undeclared\":[],\"unused\":[],\"suggestion\":\"docs/adr/0001.md\",\"rewritten\":false,\"message\":null}]}]}\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn without_strace_trace_is_exit_2_and_says_what_to_install() {
    let repo = Repo::new();
    let out = repo
        .cmd(&["trace", "--trust"])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("install it"), "{}", stderr(&out));
}

#[test]
fn a_transcript_is_traced_and_a_recipe_is_not_rewritten() {
    if !tracer() {
        return;
    }
    let repo = Repo::new();
    fs::write(
        repo.path().join("computed.toml"),
        "[recipe.one]\nloader = \"exec\"\ncmd = \"cat docs/adr/0001.md src/main.rs\"\ninputs = \"docs/adr/0001.md\"\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("DOC.md"),
        "<!-- computed transcript steps=\"cat docs/adr/0001.md ;; cat Cargo.toml\" inputs=docs/adr/0001.md name=t -->\n<!-- /computed -->\n\n<!-- computed use recipe=one name=r -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let before = repo.doc();
    let out = repo
        .cmd(&["trace", "--trust", "--write", "--only", "r"])
        .output()
        .unwrap();
    let err = stderr(&out);
    assert!(err.contains("DOC.md:4 r exec undeclared"), "{err}");
    assert!(
        err.contains("not rewritten: inputs= comes from [recipe.one] in computed.toml"),
        "{err}"
    );
    assert_eq!(repo.doc(), before, "the use opener is left alone");

    let out = repo
        .cmd(&["trace", "--trust", "--only", "t"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("DOC.md:1 t transcript undeclared\n    undeclared: Cargo.toml\n"),
        "{err}"
    );
}
