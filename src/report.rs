//! The stderr line per region, loader stderr indented beneath, the unified
//! diff on stdout for `--dry-run`, and the JSON document `--format json`
//! prints instead of both.

use std::fmt::Write as _;
use std::path::Path;

use crate::render::{Action, Mode, RegionReport};

/// Whether a region's line is shown without `-v`: fresh regions, and
/// volatile regions under `check`, are silent.
fn shown(r: &RegionReport, mode: Mode) -> bool {
    match r.action {
        Some(Action::Fresh) => false,
        Some(_) => true,
        None => mode == Mode::Check && r.state.drifted() || mode != Mode::Check,
    }
}

/// The region lines for one file, one block per region: `path:line name
/// loader state action`, with the name and state columns padded to the
/// file's widest, and loader stderr indented beneath.
pub fn regions(path: &Path, regions: &[RegionReport], mode: Mode, verbose: bool) -> Vec<String> {
    let regions: Vec<&RegionReport> = regions
        .iter()
        .filter(|r| verbose || shown(r, mode))
        .collect();
    let name_width = regions
        .iter()
        .map(|r| r.name.as_deref().map_or(0, |n| n.chars().count()))
        .max()
        .unwrap_or(0);
    let state_width = regions.iter().map(|r| state_of(r).len()).max().unwrap_or(0);
    let mut blocks = Vec::new();
    for r in regions {
        let name = r.name.as_deref().unwrap_or("");
        let mut line = format!(
            "{}:{} {name:name_width$} {} {:state_width$}",
            path.display(),
            r.line,
            r.loader,
            state_of(r)
        );
        if let Some(action) = r.action.map(|a| a.to_string())
            && !action.is_empty()
        {
            line.push(' ');
            line.push_str(&action);
        }
        let mut block = String::new();
        writeln!(block, "{}", line.trim_end()).unwrap();
        if let Some(stderr) = &r.stderr {
            for l in stderr.lines() {
                writeln!(block, "    {l}").unwrap();
            }
        }
        blocks.push(block);
    }
    blocks
}

fn state_of(r: &RegionReport) -> String {
    match r.action {
        Some(Action::Untrusted) => "untrusted".to_string(),
        Some(Action::Disallowed) => "disallowed".to_string(),
        _ => r.state.to_string(),
    }
}

/// A file-level error line, tier 2: at a line of the file, or the file itself.
pub fn error(path: &Path, line: Option<usize>, message: &str) -> String {
    match line {
        Some(line) => format!("{}:{line}: {message}\n", path.display()),
        None => format!("{}: {message}\n", path.display()),
    }
}

/// The unified diff `--dry-run` prints to stdout. `label` marks the new side
/// when it is not what `run` itself would write.
pub fn diff(path: &Path, old: &str, new: &str, label: Option<&str>) -> String {
    let name = path.display().to_string();
    let new_name = match label {
        Some(l) => format!("{name} ({l})"),
        None => name.clone(),
    };
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&name, &new_name)
        .to_string()
}

/// One file's part of the JSON document.
pub struct FileJson<'a> {
    pub path: &'a Path,
    pub error: Option<(Option<usize>, &'a str)>,
    pub regions: &'a [RegionReport],
    pub diff: Option<&'a str>,
}

/// The document `--format json` prints on stdout: the exit code and, per
/// file with something to say, its error, every region and its diff.
pub fn json(files: &[FileJson<'_>], exit: u8) -> String {
    let mut out = format!("{{\"exit\":{exit},\"files\":[");
    for (i, f) in files.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write!(out, "{{\"path\":{}", string(&f.path.display().to_string())).unwrap();
        out.push_str(",\"error\":");
        match f.error {
            Some((line, message)) => write!(
                out,
                "{{\"line\":{},\"message\":{}}}",
                line.map_or("null".to_string(), |l| l.to_string()),
                string(message)
            )
            .unwrap(),
            None => out.push_str("null"),
        }
        out.push_str(",\"regions\":[");
        for (j, r) in f.regions.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            write!(
                out,
                "{{\"line\":{},\"name\":{},\"loader\":{},\"state\":{},\"action\":{},\"message\":{},\"severity\":{}}}",
                r.line,
                optional(r.name.as_deref()),
                string(&r.loader),
                string(&r.state.to_string()),
                optional(r.action.filter(|&a| a != Action::Warn).map(Action::key)),
                optional(r.stderr.as_deref()),
                optional((r.action == Some(Action::Warn)).then_some("warn")),
            )
            .unwrap();
        }
        write!(out, "],\"diff\":{}}}", optional(f.diff)).unwrap();
    }
    out.push_str("]}\n");
    out
}

pub(crate) fn optional(s: Option<&str>) -> String {
    s.map_or("null".to_string(), string)
}

/// A JSON string literal.
pub(crate) fn string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::State;

    #[test]
    fn json_escapes_and_nulls() {
        let regions = [RegionReport {
            line: 3,
            name: Some("a\"b".into()),
            loader: "exec".into(),
            state: State::Stale,
            action: Some(Action::Failed),
            stderr: Some("x\n\ty\u{1}".into()),
        }];
        let doc = json(
            &[FileJson {
                path: Path::new("d.md"),
                error: None,
                regions: &regions,
                diff: None,
            }],
            1,
        );
        assert_eq!(
            doc,
            "{\"exit\":1,\"files\":[{\"path\":\"d.md\",\"error\":null,\"regions\":[{\"line\":3,\"name\":\"a\\\"b\",\"loader\":\"exec\",\"state\":\"stale\",\"action\":\"failed\",\"message\":\"x\\n\\ty\\u0001\",\"severity\":null}],\"diff\":null}]}\n"
        );
    }
}
