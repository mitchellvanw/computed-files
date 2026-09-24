//! A throwaway repository to run `computed` in, shared by the loader and
//! sink tests.

#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use assert_cmd::prelude::*;
use computed::loader::{Ctx, LoadError, Loaded, Production};
use computed::marker::{self, Region, Segment};
use computed::render::Loaders;

pub struct Sandbox {
    dir: tempfile::TempDir,
    config: tempfile::TempDir,
}

impl Sandbox {
    /// An empty repository: a `.git` directory and nothing else.
    pub fn new() -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        Sandbox {
            dir,
            config: tempfile::tempdir().unwrap(),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, rel: &str, text: &str) -> &Sandbox {
        let path = self.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        self
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path().join(rel)).unwrap()
    }

    /// `computed` with `args`, run in the repository with an empty trust store.
    pub fn run(&self, args: &[&str]) -> Output {
        Command::cargo_bin("computed")
            .unwrap()
            .current_dir(self.path())
            .env("XDG_CONFIG_HOME", self.config.path())
            .args(args)
            .output()
            .unwrap()
    }

    /// The production loaders for a template at `rel`.
    pub fn loaders(&self, rel: &str) -> Production {
        Production::new(Ctx::for_template(&self.path().join(rel)))
    }

    /// `load` for one opener in a template at `template`.
    pub fn load(&self, template: &str, opener: &str) -> Result<Loaded, LoadError> {
        self.loaders(template).load(&region(opener))
    }

    /// `snapshot` for one opener in a template at `template`.
    pub fn snapshot(&self, template: &str, opener: &str) -> Result<Vec<u8>, LoadError> {
        self.loaders(template)
            .snapshot(&region(opener))
            .map(|s| s.expect("not volatile"))
    }
}

/// The region an opener line opens, with an empty body.
pub fn region(opener: &str) -> Region {
    match marker::parse(&format!("{opener}\n<!-- /computed -->\n"))
        .unwrap()
        .segments
        .remove(0)
    {
        Segment::Region(r) => r,
        Segment::Prose(_) => unreachable!(),
    }
}

/// The parse error an opener line gives.
pub fn parse_error(opener: &str) -> String {
    marker::parse(&format!("{opener}\n<!-- /computed -->\n"))
        .expect_err(opener)
        .message
}

pub fn code(out: &Output) -> Option<i32> {
    out.status.code()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The error message of a hard loader error.
pub fn hard(result: Result<impl std::fmt::Debug, LoadError>) -> String {
    match result {
        Err(LoadError::Hard(m)) => m,
        other => panic!("expected a hard error, got {other:?}"),
    }
}
