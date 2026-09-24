//! Recipes: a `use` region expands to the opener a `computed.toml` names.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use assert_cmd::prelude::*;

struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new(config: &str) -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".git")).unwrap();
        fs::create_dir_all(r.join("docs")).unwrap();
        fs::write(r.join("part.md"), "root part\n").unwrap();
        fs::write(r.join("docs/part.md"), "docs part\n").unwrap();
        fs::write(r.join("computed.toml"), config).unwrap();
        Repo { dir }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn write(&self, rel: &str, text: &str) {
        fs::write(self.path().join(rel), text).unwrap();
    }
    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path().join(rel)).unwrap()
    }
    fn cmd(&self, args: &[&str]) -> Output {
        let mut c = Command::cargo_bin("computed").unwrap();
        c.current_dir(self.path())
            .env("XDG_CONFIG_HOME", self.path().join(".config"))
            .args(args);
        c.output().unwrap()
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const PART: &str = "[recipe.part]\nloader = \"file\"\nsrc = \"part.md\"\n";
const USE: &str = "<!-- computed use recipe=part name=p -->\n<!-- /computed -->\n";

#[test]
fn a_recipe_reads_relative_to_each_template_that_uses_it() {
    let repo = Repo::new(PART);
    repo.write("doc.md", USE);
    repo.write("docs/doc.md", USE);
    assert_eq!(repo.cmd(&["run"]).status.code(), Some(1));
    let root = repo.read("doc.md");
    assert!(root.contains("\nroot part\n"), "{root}");
    assert!(
        root.starts_with("<!-- computed use recipe=part name=p | do not edit; run computed -->\n"),
        "the file keeps the opener as written: {root}"
    );
    assert!(repo.read("docs/doc.md").contains("\ndocs part\n"));
    let out = repo.cmd(&["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.cmd(&["run"]).status.code(), Some(0));
}

#[test]
fn a_use_region_has_the_sums_of_its_expansion_written_inline() {
    let repo = Repo::new(PART);
    repo.write("a.md", USE);
    repo.write(
        "b.md",
        "<!-- computed file src=part.md name=p -->\n<!-- /computed -->\n",
    );
    repo.cmd(&["run"]);
    let closer = |f: &str| repo.read(f).lines().last().unwrap().to_string();
    assert_eq!(closer("a.md"), closer("b.md"));
}

#[test]
fn a_changed_recipe_makes_its_regions_stale() {
    let repo = Repo::new(PART);
    repo.write("doc.md", USE);
    repo.cmd(&["run"]);
    repo.write("computed.toml", &format!("{PART}as = \"fence\"\n"));
    let out = repo.cmd(&["check"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "doc.md:1 p file stale\n");
    repo.cmd(&["run"]);
    assert!(repo.read("doc.md").contains("```\nroot part\n```\n"));
}

#[test]
fn a_region_may_add_common_attributes_and_not_loader_ones() {
    let repo = Repo::new(PART);
    repo.write(
        "doc.md",
        "<!-- computed use recipe=part name=p as=fence lang=text max-lines=1 -->\n<!-- /computed -->\n",
    );
    repo.cmd(&["run"]);
    assert!(repo.read("doc.md").contains("```text\nroot part\n```\n"));
    repo.write(
        "bad.md",
        "<!-- computed use recipe=part src=other.md -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["check", "bad.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("unknown attribute src= for loader use"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_recipe_that_cannot_be_expanded_skips_only_its_region() {
    let repo = Repo::new(PART);
    repo.write(
        "doc.md",
        "<!-- computed use recipe=missing name=m -->\n<!-- /computed -->\n\n<!-- computed use recipe=part name=p -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["run"]);
    assert_eq!(out.status.code(), Some(2));
    let err = stderr(&out);
    assert!(
        err.contains("doc.md:1 m use error") && err.contains("[recipe.missing]"),
        "{err}"
    );
    assert!(
        repo.read("doc.md").contains("\nroot part\n"),
        "the other region renders"
    );
    let out = repo.cmd(&["check"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn a_malformed_computed_toml_is_a_hard_error_naming_it() {
    for (config, needle) in [
        (
            "[recipe.part]\nloader = \"file\"\nsrc = \"part.md\"\nbogus = \"x\"\n",
            "unknown attribute bogus=",
        ),
        (
            "[recipes.part]\nloader = \"file\"\n",
            "unknown key \"recipes\"",
        ),
        ("[recipe.part]\nsrc = \"part.md\"\n", "needs loader="),
        (
            "[recipe.part]\nloader = \"use\"\nrecipe = \"part\"\n",
            "another recipe",
        ),
        (
            "[recipe.part]\nloader = \"file\"\nsrc = \"part.md\"\nname = \"x\"\n",
            "name=",
        ),
        (
            "[recipe.part]\nloader = \"file\"\nsrc = [\"a\"]\n",
            "expected a string",
        ),
        ("[recipe.part\n", "computed.toml"),
    ] {
        let repo = Repo::new(config);
        repo.write("doc.md", USE);
        let out = repo.cmd(&["check"]);
        assert_eq!(out.status.code(), Some(2), "{config}");
        assert!(stderr(&out).contains(needle), "{config}: {}", stderr(&out));
    }
}

#[test]
fn an_exec_recipe_needs_trust() {
    let repo = Repo::new(
        "[recipe.date]\nloader = \"exec\"\ncmd = \"cat part.md\"\ninputs = \"part.md\"\n",
    );
    repo.write(
        "doc.md",
        "<!-- computed use recipe=date name=d -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["run"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("doc.md:1 d exec untrusted"),
        "{}",
        stderr(&out)
    );
    assert_eq!(repo.cmd(&["run", "--trust"]).status.code(), Some(1));
    assert!(repo.read("doc.md").contains("\nroot part\n"));
}

#[test]
fn a_volatile_flag_is_set_with_true() {
    let repo = Repo::new("[recipe.now]\nloader = \"exec\"\ncmd = \"echo now\"\nvolatile = true\n");
    repo.write(
        "doc.md",
        "<!-- computed use recipe=now name=n -->\n<!-- /computed -->\n",
    );
    assert_eq!(repo.cmd(&["run", "--trust"]).status.code(), Some(1));
    assert!(repo.read("doc.md").contains("\nnow\n"));
}

#[test]
fn the_nearest_computed_toml_up_to_the_repository_root_is_read() {
    let repo = Repo::new(PART);
    fs::create_dir_all(repo.path().join("docs/deep")).unwrap();
    repo.write("docs/deep/part.md", "deep part\n");
    repo.write("docs/deep/doc.md", USE);
    repo.write(
        "docs/computed.toml",
        "[recipe.part]\nloader = \"file\"\nsrc = \"part.md\"\nas = \"fence\"\n",
    );
    repo.cmd(&["run"]);
    assert!(
        repo.read("docs/deep/doc.md")
            .contains("```\ndeep part\n```\n")
    );

    // A computed.toml above the repository root is not the repository's.
    let outer = tempfile::tempdir().unwrap();
    fs::write(outer.path().join("computed.toml"), PART).unwrap();
    let inner = outer.path().join("repo");
    fs::create_dir_all(inner.join(".git")).unwrap();
    fs::write(inner.join("part.md"), "x\n").unwrap();
    fs::write(inner.join("doc.md"), USE).unwrap();
    let out = Command::cargo_bin("computed")
        .unwrap()
        .current_dir(&inner)
        .env("XDG_CONFIG_HOME", inner.join(".config"))
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("no computed.toml"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn expansion_records_computed_toml_as_read_so_run_settles_on_it() {
    use computed::loader::{Ctx, Production};
    use computed::marker::{self, Segment};
    let repo = Repo::new(PART);
    repo.write("doc.md", USE);
    let mut file = marker::parse(USE).unwrap();
    let mut loaders = Production::new(Ctx::for_template(&repo.path().join("doc.md")));
    loaders.expand_recipes(&mut file);
    let toml = repo.path().join("computed.toml").canonicalize().unwrap();
    assert!(loaders.read().contains(&toml));
    let Segment::Region(r) = &file.segments[0] else {
        panic!()
    };
    assert_eq!(r.opener.loader, "file");
    assert_eq!(r.opener.attr("src"), Some("part.md"));
}

#[test]
fn a_value_recipe_reads_one_key() {
    let repo = Repo::new(
        "[recipe.version]\nloader = \"value\"\nsrc = \"Cargo.toml\"\nkey = \"package.version\"\n",
    );
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"1.2.3\"\n",
    );
    repo.write(
        "doc.md",
        "<!-- computed use recipe=version -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["run"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        repo.read("doc.md").contains("\n1.2.3\n"),
        "{}",
        repo.read("doc.md")
    );
    assert_eq!(repo.cmd(&["check"]).status.code(), Some(0));
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"y\"\nversion = \"1.2.3\"\n",
    );
    assert_eq!(
        repo.cmd(&["check"]).status.code(),
        Some(0),
        "only the key counts"
    );
    repo.write("Cargo.toml", "[package]\nversion = \"2.0.0\"\n");
    assert_eq!(stderr(&repo.cmd(&["check"])), "doc.md:1  value stale\n");
}

#[test]
fn an_index_recipe_lists_its_globs_with_titles() {
    let repo = Repo::new(
        "[recipe.docs]\nloader = \"index\"\nsrc = \"docs/*.md\"\ntitle = \"h1\"\nmax-lines = 1\n",
    );
    repo.write("docs/a.md", "# Alpha\n");
    repo.write("docs/b.md", "# Beta\n");
    repo.write(
        "index.md",
        "<!-- computed use recipe=docs name=i -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["run"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let text = repo.read("index.md");
    assert!(
        text.contains("\n- [Alpha](docs/a.md)\n… 2 more lines\n"),
        "{text}"
    );
    assert_eq!(repo.cmd(&["check"]).status.code(), Some(0));
    repo.write("docs/b.md", "# Bravo\n");
    assert_eq!(stderr(&repo.cmd(&["check"])), "index.md:1 i index stale\n");
}

#[test]
fn a_table_recipe_carries_delim_and_a_region_may_override_it() {
    let repo = Repo::new(
        "[recipe.deps]\nloader = \"file\"\nsrc = \"deps.tsv\"\nas = \"table\"\ndelim = \"tab\"\n",
    );
    repo.write("deps.tsv", "crate\tversion\nclap\t4\n");
    repo.write("deps.csv", "crate,version\nclap,4\n");
    repo.write(
        "a.md",
        "<!-- computed use recipe=deps -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["run"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        repo.read("a.md")
            .contains("| crate | version |\n| ----- | ------- |\n| clap  | 4       |\n"),
        "{}",
        repo.read("a.md")
    );
    // The region's `delim=` replaces the recipe's, as any common attribute does.
    repo.write(
        "computed.toml",
        "[recipe.deps]\nloader = \"file\"\nsrc = \"deps.csv\"\nas = \"table\"\ndelim = \"tab\"\n",
    );
    repo.write(
        "b.md",
        "<!-- computed use recipe=deps delim=comma -->\n<!-- /computed -->\n",
    );
    let out = repo.cmd(&["run", "b.md"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        repo.read("b.md").contains("| clap  | 4       |\n"),
        "{}",
        repo.read("b.md")
    );
    // Without a table to shape, `delim=` is still refused.
    repo.write(
        "computed.toml",
        "[recipe.deps]\nloader = \"file\"\nsrc = \"deps.csv\"\n",
    );
    let out = repo.cmd(&["check", "b.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("applies only with as=table"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_toc_recipe_lists_the_using_templates_headings() {
    let repo = Repo::new("[recipe.contents]\nloader = \"toc\"\nmax = 2\n");
    repo.write(
        "doc.md",
        "# Doc\n\n<!-- computed use recipe=contents -->\n<!-- /computed -->\n\n## One\n\n## Two\n",
    );
    let out = repo.cmd(&["run"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        repo.read("doc.md")
            .contains("\n- [One](#one)\n- [Two](#two)\n"),
        "{}",
        repo.read("doc.md")
    );
    assert_eq!(repo.cmd(&["run"]).status.code(), Some(0));
}
