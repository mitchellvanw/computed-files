//! The stderr line per region, loader stderr indented beneath, and the
//! unified diff on stdout for `--dry-run`.

use std::fmt::Write as _;
use std::path::Path;

use crate::render::{Action, Mode, RegionReport, Rendered};

/// Whether a region's line is shown without `-v`: fresh regions, and
/// volatile regions under `check`, are silent.
fn shown(r: &RegionReport, mode: Mode) -> bool {
    match r.action {
        Some(Action::Fresh) => false,
        Some(_) => true,
        None => mode == Mode::Check && r.state.drifted() || mode != Mode::Check,
    }
}

/// The region lines for one file: `path:line name loader state action`,
/// with the name and state columns padded to the file's widest.
pub fn regions(path: &Path, rendered: &Rendered, mode: Mode, verbose: bool) -> String {
    let regions: Vec<&RegionReport> = rendered
        .regions()
        .iter()
        .filter(|r| verbose || shown(r, mode))
        .collect();
    let name_width = regions
        .iter()
        .map(|r| r.name.as_deref().map_or(0, str::len))
        .max()
        .unwrap_or(0);
    let state_width = regions.iter().map(|r| state_of(r).len()).max().unwrap_or(0);
    let mut out = String::new();
    for r in regions {
        let name = r.name.as_deref().unwrap_or("");
        let mut line = format!(
            "{}:{} {name:name_width$} {} {:state_width$}",
            path.display(),
            r.line,
            r.loader,
            state_of(r)
        );
        if let Some(action) = r.action {
            let action = action.to_string();
            if !action.is_empty() {
                line.push(' ');
                line.push_str(&action);
            }
        }
        writeln!(out, "{}", line.trim_end()).unwrap();
        if let Some(stderr) = &r.stderr {
            for l in stderr.lines() {
                writeln!(out, "    {l}").unwrap();
            }
        }
    }
    out
}

fn state_of(r: &RegionReport) -> String {
    match r.action {
        Some(Action::Untrusted) => "untrusted".to_string(),
        _ => r.state.to_string(),
    }
}

/// A file-level error line, tier 2.
pub fn error(path: &Path, line: usize, message: &str) -> String {
    format!("{}:{line}: {message}\n", path.display())
}

/// The unified diff `--dry-run` prints to stdout.
pub fn diff(path: &Path, old: &str, new: &str) -> String {
    let name = path.display().to_string();
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&name, &name)
        .to_string()
}
