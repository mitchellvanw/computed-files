//! `computed stats`: how much of each file is computed. Per region its body
//! lines, bytes and an estimate of its tokens; per file the bytes in region
//! bodies against the whole. Read from the files alone: no loader runs and
//! no snapshot is taken.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::marker::{self, Segment};
use crate::report;

/// One region's body as it sits in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionStats {
    pub line: usize,
    /// The opener's column for a region inside a line.
    pub column: Option<usize>,
    pub name: Option<String>,
    /// The loader as the opener names it; a `use` region is counted as written.
    pub loader: String,
    pub lines: usize,
    pub bytes: usize,
}

/// One file: its size, its regions in line order, or why it was not counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStats {
    pub path: PathBuf,
    pub bytes: usize,
    pub regions: Vec<RegionStats>,
    /// A file that could not be read or parsed, at a line or on the file.
    pub error: Option<(Option<usize>, String)>,
}

impl FileStats {
    fn region_bytes(&self) -> usize {
        self.regions.iter().map(|r| r.bytes).sum()
    }
}

/// The token estimate: a quarter of the bytes, rounded up. A rough rule for
/// English and code under common tokenisers, not a count.
pub fn tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

/// Reads and parses one file. A file with no region has nothing to count
/// and is `None`, whatever its encoding.
pub fn file(path: &Path) -> Option<FileStats> {
    let failed = |line, message: String| {
        Some(FileStats {
            path: path.to_path_buf(),
            bytes: 0,
            regions: Vec::new(),
            error: Some((line, message)),
        })
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return failed(None, format!("unreadable: {e}")),
    };
    let syntax = marker::Syntax::for_path(path);
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) if marker::has_marker(&String::from_utf8_lossy(e.as_bytes()), syntax) => {
            return failed(None, "not UTF-8".to_string());
        }
        Err(_) => return None,
    };
    let parsed = match marker::parse_as(&text, syntax) {
        Ok(p) => p,
        Err(e) => return failed(Some(e.line), e.message),
    };
    let regions: Vec<RegionStats> = parsed
        .segments
        .iter()
        .filter_map(|s| match s {
            Segment::Region(r) => Some(RegionStats {
                line: r.line,
                column: r.column,
                name: r.opener.name.clone(),
                loader: r.opener.loader.clone(),
                // A region inside a line takes no line of its own.
                lines: match r.column {
                    Some(_) => 0,
                    None => r.body.split_inclusive('\n').count(),
                },
                bytes: r.body.len(),
            }),
            Segment::Prose(_) => None,
        })
        .collect();
    (!regions.is_empty()).then(|| FileStats {
        path: path.to_path_buf(),
        bytes: text.len(),
        regions,
        error: None,
    })
}

/// Counts every file and prints the report: text on stdout, a file that
/// could not be counted on stderr, or one JSON document on stdout. The exit
/// tier is 2 when a file could not be counted, else 0.
pub fn run(paths: &[PathBuf], json: bool) -> u8 {
    let files: Vec<FileStats> = paths.iter().filter_map(|p| file(p)).collect();
    let exit = if files.iter().any(|f| f.error.is_some()) {
        2
    } else {
        0
    };
    if json {
        print!("{}", self::json(&files, exit));
    } else {
        for f in &files {
            if let Some((line, message)) = &f.error {
                eprint!("{}", report::error(&f.path, *line, message));
            }
        }
        print!("{}", text(&files));
    }
    exit
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {one}s")
    }
}

fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "0.0".to_string();
    }
    format!("{:.1}", part as f64 * 100.0 / whole as f64)
}

/// `bytes computed (share), ~tokens` for a part of a whole.
fn share(part: usize, whole: usize) -> String {
    format!(
        "{part} of {whole} bytes computed ({}%), ~{} of ~{} tokens",
        percent(part, whole),
        tokens(part),
        tokens(whole)
    )
}

/// One line per region, `path:line name loader lines bytes ~tokens`, the
/// name and loader columns padded to the file's widest; a line per file;
/// and the totals. Files that could not be counted are left out.
pub fn text(files: &[FileStats]) -> String {
    let mut out = String::new();
    let counted: Vec<&FileStats> = files.iter().filter(|f| f.error.is_none()).collect();
    for f in &counted {
        let name_width = f
            .regions
            .iter()
            .map(|r| r.name.as_deref().map_or(0, |n| n.chars().count()))
            .max()
            .unwrap_or(0);
        let loader_width = f.regions.iter().map(|r| r.loader.len()).max().unwrap_or(0);
        for r in &f.regions {
            writeln!(
                out,
                "{}:{} {:name_width$} {:loader_width$} {} {} ~{}",
                f.path.display(),
                crate::marker::place(r.line, r.column),
                r.name.as_deref().unwrap_or(""),
                r.loader,
                plural(r.lines, "line"),
                plural(r.bytes, "byte"),
                plural(tokens(r.bytes), "token"),
            )
            .unwrap();
        }
        writeln!(
            out,
            "{}: {}",
            f.path.display(),
            share(f.region_bytes(), f.bytes)
        )
        .unwrap();
    }
    let regions: usize = counted.iter().map(|f| f.regions.len()).sum();
    let part: usize = counted.iter().map(|f| f.region_bytes()).sum();
    let whole: usize = counted.iter().map(|f| f.bytes).sum();
    writeln!(
        out,
        "{} in {}: {}",
        plural(regions, "region"),
        plural(counted.len(), "file"),
        share(part, whole)
    )
    .unwrap();
    out
}

/// `bytes`, `region_bytes`, `percent`, `tokens` and `region_tokens` for a
/// part of a whole, as JSON members.
fn json_share(part: usize, whole: usize) -> String {
    format!(
        "\"bytes\":{whole},\"region_bytes\":{part},\"percent\":{},\"tokens\":{},\"region_tokens\":{}",
        percent(part, whole),
        tokens(whole),
        tokens(part)
    )
}

/// `{"exit", "files": [{path, error, bytes, region_bytes, percent, tokens,
/// region_tokens, regions: [{line, name, loader, lines, bytes, tokens}]}],
/// "totals": {files, regions, bytes, region_bytes, percent, tokens,
/// region_tokens}}`. A file that could not be counted has its error and no
/// regions, and is left out of the totals.
pub fn json(files: &[FileStats], exit: u8) -> String {
    let mut out = format!("{{\"exit\":{exit},\"files\":[");
    for (i, f) in files.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let error = match &f.error {
            Some((line, message)) => format!(
                "{{\"line\":{},\"message\":{}}}",
                line.map_or("null".to_string(), |l| l.to_string()),
                report::string(message)
            ),
            None => "null".to_string(),
        };
        write!(
            out,
            "{{\"path\":{},\"error\":{error},{},\"regions\":[",
            report::string(&f.path.display().to_string()),
            json_share(f.region_bytes(), f.bytes)
        )
        .unwrap();
        for (j, r) in f.regions.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            write!(
                out,
                "{{\"line\":{},\"name\":{},\"loader\":{},\"lines\":{},\"bytes\":{},\"tokens\":{}}}",
                r.line,
                r.name.as_deref().map_or("null".to_string(), report::string),
                report::string(&r.loader),
                r.lines,
                r.bytes,
                tokens(r.bytes)
            )
            .unwrap();
        }
        out.push_str("]}");
    }
    let counted: Vec<&FileStats> = files.iter().filter(|f| f.error.is_none()).collect();
    writeln!(
        out,
        "],\"totals\":{{\"files\":{},\"regions\":{},{}}}}}",
        counted.len(),
        counted.iter().map(|f| f.regions.len()).sum::<usize>(),
        json_share(
            counted.iter().map(|f| f.region_bytes()).sum(),
            counted.iter().map(|f| f.bytes).sum()
        )
    )
    .unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_a_quarter_of_the_bytes_rounded_up() {
        assert_eq!(tokens(0), 0);
        assert_eq!(tokens(1), 1);
        assert_eq!(tokens(8), 2);
        assert_eq!(tokens(9), 3);
    }

    #[test]
    fn an_empty_set_of_files_totals_zero() {
        assert_eq!(
            text(&[]),
            "0 regions in 0 files: 0 of 0 bytes computed (0.0%), ~0 of ~0 tokens\n"
        );
        assert_eq!(
            json(&[], 0),
            "{\"exit\":0,\"files\":[],\"totals\":{\"files\":0,\"regions\":0,\"bytes\":0,\"region_bytes\":0,\"percent\":0.0,\"tokens\":0,\"region_tokens\":0}}\n"
        );
    }
}
