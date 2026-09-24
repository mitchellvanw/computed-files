//! The exec `sandbox` flag end to end, enforced for real: a declared input
//! reads, an undeclared file does not, writes land only in `TMPDIR`, the
//! network is closed, and trust still applies. Skipped, with a note, where
//! this machine cannot sandbox (a Linux kernel without Landlock).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use assert_cmd::prelude::*;

struct Repo {
    dir: tempfile::TempDir,
    config: tempfile::TempDir,
}

impl Repo {
    fn new(template: &str) -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::create_dir_all(r.join("docs")).unwrap();
        fs::write(r.join("docs/a.txt"), "declared\n").unwrap();
        fs::write(r.join("secret.txt"), "undeclared\n").unwrap();
        fs::write(r.join("DOC.md"), template).unwrap();
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
    fn run(&self) -> Output {
        self.cmd(&["run", "--trust"]).output().unwrap()
    }
    fn doc(&self) -> String {
        fs::read_to_string(self.path().join("DOC.md")).unwrap()
    }
}

fn region(cmd: &str) -> String {
    format!(
        "<!-- computed exec cmd=\"{cmd}\" inputs=docs/a.txt sandbox name=r -->\n<!-- /computed -->\n"
    )
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Whether this machine can sandbox; says why not when it cannot.
fn sandboxable() -> bool {
    let repo = Repo::new(&region("true"));
    let out = repo.run();
    let err = stderr(&out);
    if err.contains("Landlock is not available") || err.contains("seccomp") {
        eprintln!("skipped: {err}");
        return false;
    }
    assert_eq!(out.status.code(), Some(1), "{err}");
    true
}

#[test]
fn a_declared_input_reads_and_an_undeclared_file_fails_the_region() {
    if !sandboxable() {
        return;
    }
    let repo = Repo::new(&region("cat docs/a.txt; ls docs"));
    let out = repo.run();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("DOC.md:1 r exec unrendered written"),
        "{}",
        stderr(&out)
    );
    assert!(repo.doc().contains("declared\na.txt\n"), "{}", repo.doc());

    let repo = Repo::new(&region("cat docs/a.txt secret.txt"));
    let out = repo.run();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("DOC.md:1 r exec unrendered failed; body kept"),
        "{err}"
    );
    assert!(
        err.contains("secret.txt: Operation not permitted")
            || err.contains("secret.txt: Permission denied"),
        "{err}"
    );
    assert!(!repo.doc().contains("undeclared"));
}

#[test]
fn a_system_or_path_directory_above_the_repository_does_not_open_it() {
    if !sandboxable() {
        return;
    }
    // A `PATH` entry that holds the repository, as `/usr` holds a checkout
    // in `/usr/src/app`: the programs beside it still run, but the
    // repository's undeclared files stay closed.
    let repo = Repo::new(&region("cat secret.txt"));
    let above = repo.path().canonicalize().unwrap();
    let above = above.parent().unwrap();
    let out = repo
        .cmd(&["run", "--trust"])
        .env("PATH", format!("{}:/usr/bin:/bin", above.display()))
        .output()
        .unwrap();
    let err = stderr(&out);
    assert!(err.contains("r exec unrendered failed; body kept"), "{err}");
    assert!(!repo.doc().contains("undeclared"), "{}", repo.doc());
}

#[test]
fn writes_land_only_in_tmpdir() {
    if !sandboxable() {
        return;
    }
    let repo = Repo::new(&region("echo scratch > $TMPDIR/f && cat $TMPDIR/f"));
    let out = repo.run();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(repo.doc().contains("scratch"), "{}", stderr(&out));

    let repo = Repo::new(&region("echo x > out.txt"));
    let out = repo.run();
    assert!(
        stderr(&out).contains("failed; body kept"),
        "{}",
        stderr(&out)
    );
    assert!(!repo.path().join("out.txt").exists());
}

#[test]
fn the_network_is_closed() {
    if !sandboxable() {
        return;
    }
    let curl = ["/usr/bin/curl", "/bin/curl"]
        .into_iter()
        .find(|c| Path::new(c).exists());
    let Some(curl) = curl else {
        eprintln!("skipped: no curl");
        return;
    };
    let repo = Repo::new(&region(&format!(
        "{curl} -sS -m 5 -o /dev/null http://1.1.1.1/"
    )));
    let out = repo.run();
    let err = stderr(&out);
    assert!(
        err.contains("DOC.md:1 r exec unrendered failed; body kept"),
        "{err}"
    );
}

#[test]
fn a_sandboxed_region_still_needs_trust() {
    let repo = Repo::new(&region("cat docs/a.txt"));
    let out = repo.cmd(&["run"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("r exec untrusted"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn the_flag_is_part_of_the_opener_and_needs_inputs() {
    if !sandboxable() {
        return;
    }
    let repo = Repo::new(
        "<!-- computed exec cmd=\"cat docs/a.txt\" inputs=docs/a.txt name=r -->\n<!-- /computed -->\n",
    );
    repo.run();
    assert_eq!(
        repo.cmd(&["check"]).output().unwrap().status.code(),
        Some(0)
    );
    let sandboxed = repo.doc().replace("name=r", "sandbox name=r");
    fs::write(repo.path().join("DOC.md"), sandboxed).unwrap();
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("r exec stale"), "{}", stderr(&out));
    repo.run();
    assert_eq!(
        repo.cmd(&["check"]).output().unwrap().status.code(),
        Some(0)
    );

    let repo = Repo::new("<!-- computed exec cmd=date volatile sandbox -->\n<!-- /computed -->\n");
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("sandbox needs inputs="),
        "{}",
        stderr(&out)
    );
}

#[test]
fn doctor_runs_a_sandboxed_region_inside_the_sandbox() {
    if !sandboxable() {
        return;
    }
    let repo = Repo::new(&region("cat docs/a.txt"));
    let out = repo.cmd(&["-v", "doctor", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("r exec deterministic"),
        "{}",
        stderr(&out)
    );
    let repo = Repo::new(&region("cat secret.txt"));
    let out = repo.cmd(&["doctor", "--trust"]).output().unwrap();
    assert!(stderr(&out).contains("r exec failed"), "{}", stderr(&out));
}

#[test]
fn an_exec_recipe_with_sandbox_true_runs_sandboxed() {
    if !sandboxable() {
        return;
    }
    let repo = Repo::new(
        "<!-- computed use recipe=a name=a -->\n<!-- /computed -->\n\n<!-- computed use recipe=secret name=s -->\n<!-- /computed -->\n",
    );
    fs::write(
        repo.path().join("computed.toml"),
        "[recipe.a]\nloader = \"exec\"\ncmd = \"cat docs/a.txt\"\ninputs = \"docs/a.txt\"\nsandbox = true\n\n\
         [recipe.secret]\nloader = \"exec\"\ncmd = \"cat secret.txt\"\ninputs = \"docs/a.txt\"\nsandbox = true\n",
    )
    .unwrap();
    let out = repo.run();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(repo.doc().contains("\ndeclared\n"), "{}", repo.doc());
    assert!(!repo.doc().contains("undeclared"), "{}", repo.doc());
    assert!(
        stderr(&out).contains("DOC.md:4 s exec unrendered failed; body kept"),
        "{}",
        stderr(&out)
    );
}

fn transcript(attrs: &str) -> String {
    format!("<!-- computed transcript {attrs} name=t -->\n<!-- /computed -->\n")
}

#[test]
fn a_sandboxed_transcript_reads_its_inputs_and_writes_only_its_temporary_places() {
    if !sandboxable() {
        return;
    }
    let repo = Repo::new(&transcript(
        "steps=\"cat docs/a.txt ;; cat secret.txt ;; echo $? ;; touch here.txt ;; echo x > $TMPDIR/t ;; cat $TMPDIR/t\" inputs=docs/a.txt sandbox",
    ));
    let out = repo.run();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let doc = repo.doc();
    assert!(doc.contains("$ cat docs/a.txt\ndeclared\n"), "{doc}");
    assert!(!doc.contains("\nundeclared\n"), "{doc}");
    assert!(
        doc.contains("$ echo $?\n1\n"),
        "the undeclared read fails: {doc}"
    );
    assert!(
        !repo.path().join("here.txt").exists(),
        "no write into the repository"
    );
    assert!(doc.contains("$ cat $TMPDIR/t\nx\n"), "{doc}");

    // workdir=tmp is the steps' own to write in.
    let repo = Repo::new(&transcript(
        "steps=\"touch made.txt ;; ls ;; cat $COMPUTED_ROOT/docs/a.txt\" inputs=docs/a.txt workdir=tmp sandbox",
    ));
    let out = repo.run();
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        repo.doc()
            .contains("$ ls\nmade.txt\n$ cat $COMPUTED_ROOT/docs/a.txt\ndeclared\n"),
        "{}",
        repo.doc()
    );
}

#[test]
fn a_sandboxed_transcript_needs_inputs_and_no_copy() {
    for (attrs, needle) in [
        ("steps=ls volatile sandbox", "sandbox needs inputs="),
        (
            "steps=ls inputs=docs/a.txt workdir=copy sandbox",
            "sandbox and workdir=copy",
        ),
    ] {
        let repo = Repo::new(&transcript(attrs));
        let out = repo.run();
        assert_eq!(out.status.code(), Some(2), "{attrs}");
        assert!(stderr(&out).contains(needle), "{}", stderr(&out));
    }
}
