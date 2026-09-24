//! `dupes`: verbatim blocks copied between Markdown files, or within one,
//! each with the `file` region that would replace a copy with an include of
//! the original. A lint: exit 1 when it finds any.
//!
//! A block is a run of consecutive non-blank lines, compared with trailing
//! whitespace taken off and blank lines skipped, so copies that differ only
//! in spacing between paragraphs still match. Region bodies and markers are
//! already computed and never count; a region breaks a run. Lines that carry
//! no text of their own (fences, thematic breaks, table delimiter rows) may
//! sit inside a block but do not count towards `--min-lines`. Candidates
//! are found with a rolling hash over windows of that many lines, confirmed
//! line by line, and grown to the longest run each pair of copies shares.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::affected;
use crate::marker::{self, Segment, Syntax};
use crate::report;
use crate::survey::{self, FileError};

/// One non-blank line outside every region.
struct Token {
    /// 1-based.
    line: usize,
    text: String,
    hash: u64,
    trivial: bool,
    /// A region stands between this line and the one before it.
    after_region: bool,
}

struct Doc {
    path: PathBuf,
    /// Every line, for finding sections.
    lines: Vec<String>,
    tokens: Vec<Token>,
}

/// A block's place in one file: the tokens `start..start + len`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Spot {
    doc: usize,
    start: usize,
    len: usize,
}

/// Lists the duplicated blocks of at least `min_lines` lines in the
/// Markdown files under `paths`. Exit 0 when there are none, 1 when there
/// are, 2 when a file could not be read.
pub fn main(paths: &[PathBuf], min_lines: usize, json: bool) -> Result<u8, String> {
    let min = min_lines.max(1);
    let files = crate::cli::discover(paths).map_err(|e| format!("{e:#}"))?;
    let mut docs = Vec::new();
    let mut errors = Vec::new();
    for path in files {
        match doc(&path) {
            Ok(Some(d)) => docs.push(d),
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }
    let groups = groups(&docs, min);
    let exit = if !errors.is_empty() {
        2
    } else {
        u8::from(!groups.is_empty())
    };
    let mut text = String::new();
    let mut items = Vec::new();
    for spots in &groups {
        let source = *spots
            .iter()
            .min_by_key(|s| {
                let name = docs[s.doc].path.file_name().unwrap_or_default();
                (
                    name == "CLAUDE.md" || name == "AGENTS.md",
                    &docs[s.doc].path,
                    s.start,
                )
            })
            .expect("a group has spots");
        let range = |s: &Spot| {
            let t = &docs[s.doc].tokens;
            (t[s.start].line, t[s.start + s.len - 1].line)
        };
        let (a, b) = range(&source);
        let copies: Vec<(Spot, Option<String>)> = spots
            .iter()
            .filter(|s| **s != source)
            .map(|s| (*s, suggestion(&docs, &source, s)))
            .collect();
        if json {
            let copies: Vec<String> = copies
                .iter()
                .map(|(s, sug)| {
                    let (c, d) = range(s);
                    format!(
                        "{{\"path\":{},\"start\":{c},\"end\":{d},\"suggestion\":{}}}",
                        report::string(&docs[s.doc].path.display().to_string()),
                        sug.as_deref().map_or("null".to_string(), report::string)
                    )
                })
                .collect();
            items.push(format!(
                "{{\"lines\":{},\"source\":{{\"path\":{},\"start\":{a},\"end\":{b}}},\"copies\":[{}]}}",
                source.len,
                report::string(&docs[source.doc].path.display().to_string()),
                copies.join(",")
            ));
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        writeln!(text, "{} lines in {} places", source.len, spots.len()).unwrap();
        writeln!(
            text,
            "    {}:{a}-{b} source",
            docs[source.doc].path.display()
        )
        .unwrap();
        for (s, sug) in &copies {
            let (c, d) = range(s);
            let what = sug.as_deref().unwrap_or("copy");
            writeln!(text, "    {}:{c}-{d} {what}", docs[s.doc].path.display()).unwrap();
        }
    }
    if json {
        println!(
            "{{\"exit\":{exit},\"errors\":{},\"duplicates\":[{}]}}",
            affected::errors_json(&errors),
            items.join(",")
        );
    } else {
        affected::print_errors(&errors);
        print!("{text}");
    }
    Ok(exit)
}

/// A Markdown file's tokens; `None` for a file that is not text, or not
/// Markdown: a file discovery reads for its comment syntax is code, and a
/// copy in code is not a block of prose to include.
fn doc(path: &Path) -> Result<Option<Doc>, FileError> {
    if !Syntax::for_path(path).is_markdown() {
        return Ok(None);
    }
    let fail = |line, message: String| FileError {
        path: path.to_path_buf(),
        line,
        message,
    };
    let bytes = std::fs::read(path).map_err(|e| fail(None, format!("unreadable: {e}")))?;
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(None);
    };
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    // The 1-based lines regions own, markers included.
    let mut owned = BTreeSet::new();
    if text.contains("<!--") {
        let parsed = marker::parse(&text).map_err(|e| fail(Some(e.line), e.message))?;
        let mut line = 1;
        for segment in &parsed.segments {
            let count = match segment {
                Segment::Prose(p) => p.split_inclusive('\n').count(),
                Segment::Region(r) => {
                    let n = 2 + r.body.split_inclusive('\n').count();
                    owned.extend(line..line + n);
                    n
                }
            };
            line += count;
        }
    }
    let mut tokens = Vec::new();
    let mut after_region = false;
    for (i, raw) in lines.iter().enumerate() {
        if owned.contains(&(i + 1)) {
            after_region = true;
            continue;
        }
        let text = raw.trim_end();
        if text.is_empty() {
            continue;
        }
        let mut h = DefaultHasher::new();
        text.hash(&mut h);
        tokens.push(Token {
            line: i + 1,
            text: text.to_string(),
            hash: h.finish(),
            trivial: trivial(text),
            after_region: std::mem::take(&mut after_region),
        });
    }
    Ok(Some(Doc {
        path: path.to_path_buf(),
        lines,
        tokens,
    }))
}

/// A line with no text of its own: a fence, a thematic break or setext
/// underline, a table's delimiter row.
fn trivial(line: &str) -> bool {
    let t = line.trim();
    let fence = ["```", "~~~"].iter().any(|f| t.starts_with(f))
        && t.trim_start_matches(['`', '~'])
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '+');
    let rule = t.len() >= 3
        && ['-', '*', '_', '=']
            .iter()
            .any(|&c| t.chars().all(|x| x == c || x == ' '));
    let delimiter =
        t.contains('|') && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
    fence || rule || delimiter
}

/// Every duplicated block, as the spots holding it, each group sorted and
/// the groups in order of their first spot. A group whose every spot lies
/// inside a longer group's is left out.
fn groups(docs: &[Doc], min: usize) -> Vec<Vec<Spot>> {
    // Windows of `min` tokens that no region breaks, by rolling hash.
    const BASE: u64 = 1_000_003;
    let top = (1..min).fold(1u64, |p, _| p.wrapping_mul(BASE));
    let mut windows: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
    for (d, doc) in docs.iter().enumerate() {
        let t = &doc.tokens;
        let mut hash = 0u64;
        let mut run = 0;
        for i in 0..t.len() {
            if t[i].after_region {
                hash = 0;
                run = 0;
            }
            if run == min {
                hash = hash.wrapping_sub(t[i - min].hash.wrapping_mul(top));
                run -= 1;
            }
            hash = hash.wrapping_mul(BASE).wrapping_add(t[i].hash);
            run += 1;
            if run == min {
                windows.entry(hash).or_default().push((d, i + 1 - min));
            }
        }
    }
    let same = |a: (usize, usize), b: (usize, usize)| {
        let (x, y) = (&docs[a.0].tokens, &docs[b.0].tokens);
        match (x.get(a.1), y.get(b.1)) {
            (Some(x), Some(y)) => x.text == y.text,
            _ => false,
        }
    };
    // Each pair of copies, grown to the longest run the two share, found
    // once: at the window where the run starts.
    let mut blocks: BTreeMap<Vec<&str>, BTreeSet<Spot>> = BTreeMap::new();
    let mut sorted: Vec<&Vec<(usize, usize)>> = windows.values().filter(|v| v.len() > 1).collect();
    sorted.sort();
    for spots in sorted {
        for (i, &a) in spots.iter().enumerate() {
            for &b in &spots[i + 1..] {
                if a.0 == b.0 && b.1 < a.1 + min {
                    continue;
                }
                if !(0..min).all(|k| same((a.0, a.1 + k), (b.0, b.1 + k))) {
                    continue;
                }
                let starts_here = a.1 == 0
                    || b.1 == 0
                    || docs[a.0].tokens[a.1].after_region
                    || docs[b.0].tokens[b.1].after_region
                    || !same((a.0, a.1 - 1), (b.0, b.1 - 1));
                if !starts_here {
                    continue;
                }
                let mut len = min;
                while same((a.0, a.1 + len), (b.0, b.1 + len))
                    && !docs[a.0].tokens[a.1 + len].after_region
                    && !docs[b.0].tokens[b.1 + len].after_region
                    && (a.0 != b.0 || a.1 + len < b.1)
                {
                    len += 1;
                }
                let tokens = &docs[a.0].tokens[a.1..a.1 + len];
                if tokens.iter().filter(|t| !t.trivial).count() < min {
                    continue;
                }
                let key: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
                let entry = blocks.entry(key).or_default();
                entry.insert(Spot {
                    doc: a.0,
                    start: a.1,
                    len,
                });
                entry.insert(Spot {
                    doc: b.0,
                    start: b.1,
                    len,
                });
            }
        }
    }
    let all: Vec<Vec<Spot>> = blocks
        .into_values()
        .map(|s| s.into_iter().collect())
        .collect();
    let inside = |s: &Spot, o: &Spot| {
        o.len > s.len && o.doc == s.doc && o.start <= s.start && s.start + s.len <= o.start + o.len
    };
    let mut kept: Vec<Vec<Spot>> = all
        .iter()
        .filter(|g| !g.iter().all(|s| all.iter().flatten().any(|o| inside(s, o))))
        .cloned()
        .collect();
    kept.sort();
    kept
}

/// The opener that would replace the copy at `copy` with an include of
/// `source`: `section=` when the source block is one whole section whose
/// heading is the only one with that text, else `lines=`. `None` within one
/// file, which no region can include.
fn suggestion(docs: &[Doc], source: &Spot, copy: &Spot) -> Option<String> {
    if source.doc == copy.doc {
        return None;
    }
    let from = survey::anchor(&docs[copy.doc].path);
    let to = survey::anchor(&docs[source.doc].path);
    let src = survey::relative(from.parent().unwrap_or(Path::new("/")), &to);
    let doc = &docs[source.doc];
    let first = doc.tokens[source.start].line;
    let last = doc.tokens[source.start + source.len - 1].line;
    let projection = match section(&doc.lines, first) {
        Some((title, end)) if end == last => format!("section={}", quote(&title)),
        _ => format!("lines={first}-{last}"),
    };
    Some(format!(
        "<!-- computed file src={} {projection} -->",
        quote(&src.display().to_string())
    ))
}

/// When line `first` (1-based) is a heading whose text no other heading
/// has: that text, and the last non-blank line of its section, which runs
/// to the next heading of the same or a higher level.
fn section(lines: &[String], first: usize) -> Option<(String, usize)> {
    let headings = headings(lines);
    let &(_, level, ref title) = headings.iter().find(|(l, _, _)| *l == first)?;
    if headings.iter().filter(|(_, _, t)| t == title).count() > 1 {
        return None;
    }
    let end = headings
        .iter()
        .find(|(l, lv, _)| *l > first && *lv <= level)
        .map_or(lines.len(), |(l, _, _)| l - 1);
    let last = (first..=end)
        .rev()
        .find(|&l| !lines[l - 1].trim().is_empty())?;
    Some((title.clone(), last))
}

/// The ATX headings outside fenced code: 1-based line, level, text.
fn headings(lines: &[String]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut fence: Option<char> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if let Some(c) = ['`', '~']
            .into_iter()
            .find(|&c| t.starts_with(&c.to_string().repeat(3)))
        {
            fence = match fence {
                None => Some(c),
                Some(f) if f == c => None,
                other => other,
            };
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let level = t.chars().take_while(|&c| c == '#').count();
        let rest = &t[level..];
        if (1..=6).contains(&level) && (rest.is_empty() || rest.starts_with([' ', '\t'])) {
            let title = rest.trim().trim_end_matches('#').trim_end().to_string();
            out.push((i + 1, level, title));
        }
    }
    out
}

/// An attribute value as the opener grammar writes it: double-quoted when
/// it holds whitespace, `>` or `"`.
fn quote(v: &str) -> String {
    if !v.is_empty() && !v.contains([' ', '\t', '>', '"']) {
        return v.to_string();
    }
    format!("\"{}\"", v.replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_lines_carry_no_text_of_their_own() {
        for t in [
            "```",
            "```rust",
            "~~~",
            "---",
            "***",
            "_ _ _",
            "| --- | :-: |",
            "|---|",
        ] {
            assert!(trivial(t), "{t}");
        }
        for t in ["# Heading", "text", "-", "- item", "```not a fence!", "--"] {
            assert!(!trivial(t), "{t}");
        }
    }

    #[test]
    fn a_section_runs_to_the_next_heading_of_its_level_or_higher() {
        let lines: Vec<String> = "# A\n\n## B\nb\n### C\nc\n\n## D\n```\n# not\n```\n"
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(section(&lines, 3), Some(("B".to_string(), 6)));
        assert_eq!(section(&lines, 1), Some(("A".to_string(), 11)));
        assert_eq!(section(&lines, 4), None);
    }

    #[test]
    fn quote_follows_the_opener_grammar() {
        assert_eq!(quote("Install"), "Install");
        assert_eq!(quote("Getting started"), "\"Getting started\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
}
