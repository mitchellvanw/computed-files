//! `computed doctor` end to end: two runs per region, the perturbation, moved
//! output, trust, JSON, and that nothing is written.

use std::fs;
use std::path::Path;
use std::process::Command;

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
        fs::write(r.join("data.txt"), "one\n").unwrap();
        fs::write(r.join("decl.txt"), "declared\n").unwrap();
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
    fn doc(&self) -> String {
        fs::read_to_string(self.path().join("DOC.md")).unwrap()
    }
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn deterministic_regions_are_silent_and_nothing_is_written() {
    let repo = Repo::new(
        "<!-- computed tree name=layout -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"cat data.txt; pwd | wc -c\" inputs=data.txt name=cat -->\n<!-- /computed -->\n",
    );
    let before = repo.doc();
    let out = repo.cmd(&["doctor", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stderr(&out), "");
    assert_eq!(repo.doc(), before, "doctor writes nothing");
    let out = repo.cmd(&["-v", "doctor", "--trust"]).output().unwrap();
    let err = stderr(&out);
    assert!(err.contains("DOC.md:1 layout tree deterministic"), "{err}");
    assert!(err.contains("DOC.md:4 cat    exec deterministic"), "{err}");
}

#[test]
fn a_command_that_reads_its_environment_is_nondeterministic() {
    let repo = Repo::new(
        "<!-- computed exec cmd=\"echo $HOME\" volatile name=home -->\n<!-- /computed -->\n\n<!-- computed exec cmd=umask volatile name=umask -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"echo $TMPDIR\" volatile name=tmpdir -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"echo $USER\" volatile name=user -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["doctor", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    for name in ["home", "umask", "tmpdir", "user"] {
        assert!(
            err.lines()
                .any(|l| l.contains(&format!(" {name} ")) && l.contains("nondeterministic")),
            "{name}: {err}"
        );
    }
    assert!(
        err.contains("--- run") && err.contains("+++ perturbed run"),
        "{err}"
    );
}

#[test]
fn output_that_changed_without_its_inputs_has_moved_and_check_cannot_see_it() {
    let repo = Repo::new(
        "<!-- computed exec cmd=\"cat data.txt\" inputs=decl.txt name=undeclared -->\n<!-- /computed -->\n",
    );
    assert_eq!(
        repo.cmd(&["run", "--trust"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert!(repo.doc().contains("one"));
    fs::write(repo.path().join("data.txt"), "two\n").unwrap();
    let out = repo.cmd(&["check"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "check answers from the sums alone"
    );
    let before = repo.doc();
    let out = repo.cmd(&["doctor", "--trust"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("DOC.md:1 undeclared exec deterministic moved"),
        "{err}"
    );
    assert!(err.contains("-one") && err.contains("+two"), "{err}");
    assert_eq!(repo.doc(), before);
}

#[test]
fn exec_needs_trust() {
    let repo = Repo::new(
        "<!-- computed tree name=layout -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"cat data.txt\" inputs=data.txt name=cat -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["doctor"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("DOC.md:4 cat exec untrusted"), "{err}");
    assert!(!err.contains("layout"), "{err}");
    assert!(repo.cmd(&["trust"]).output().unwrap().status.success());
    let out = repo.cmd(&["doctor"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn only_and_json() {
    let repo = Repo::new(
        "<!-- computed exec cmd=\"echo $HOME\" volatile name=home -->\n<!-- /computed -->\n\n<!-- computed exec cmd=false volatile name=fails -->\n<!-- /computed -->\n",
    );
    let out = repo
        .cmd(&["--format", "json", "doctor", "--trust", "--only", "fails"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "");
    let doc = String::from_utf8(out.stdout).unwrap();
    assert!(
        doc.starts_with("{\"exit\":1,\"files\":[{\"path\":\"DOC.md\""),
        "{doc}"
    );
    assert!(
        doc.contains("\"line\":4,\"name\":\"fails\",\"loader\":\"exec\",\"verdict\":\"failed\",\"moved\":false,\"diff\":null,\"moved_diff\":null,\"message\":\"exit status 1\""),
        "{doc}"
    );
    assert!(!doc.contains("home"), "{doc}");
    let out = repo
        .cmd(&["doctor", "--trust", "--only", "nobody"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("no region is named \"nobody\""));
}
