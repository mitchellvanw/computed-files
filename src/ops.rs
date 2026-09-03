use std::fs;
use std::path::Path;

use crate::publish;
use crate::render::{self, FileReport};

pub fn trusted(root: &Path) -> bool {
    root.join(".computed-trust").exists()
}

pub struct RunOutcome {
    pub exit: i32,
    pub wrote: bool,
    pub text: String,
    pub report: FileReport,
}

pub fn run_once(root: &Path, tmpl: &Path, out: &Path, force: bool) -> RunOutcome {
    let prior = fs::read_to_string(out).ok();
    let (text, report) = render::render_file(root, tmpl, out, prior.as_deref(), force, trusted(root));
    let wrote = match publish::publish(out, &text) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("write failed: {}", e);
            false
        }
    };
    RunOutcome { exit: render::run_exit(&report), wrote, text, report }
}

pub fn check_once(root: &Path, tmpl: &Path, out: &Path) -> Option<RunOutcome> {
    let prior = fs::read_to_string(out).ok()?;
    let (_text, report) = render::render_file(root, tmpl, out, Some(&prior), false, trusted(root));
    Some(RunOutcome { exit: render::check_exit(&report), wrote: false, text: String::new(), report })
}
