//! The `toc` loader's text: a nested list of links to the headings of the
//! template's own prose, anchored as GitHub anchors them. Pure; the loader
//! reads the template.
//!
//! Only prose counts. Region bodies are left out, the toc's own included,
//! so rendering a region never moves the toc, and a heading inside a fenced
//! code block is not a heading. A heading that holds a region inside its
//! line reads as GitHub shows it: the markers dropped, the body kept. The
//! snapshot is the listed headings with their anchors, which is everything
//! the list depends on.

use std::collections::HashMap;

use crate::marker::{self, Segment};
use crate::project::{self, Heading};

/// The file as a reader sees its lines: each region inside a line is its
/// body alone, its markers dropped.
fn shown(parsed: &marker::File) -> String {
    let mut out = String::new();
    for s in &parsed.segments {
        match s {
            Segment::Prose(p) => out.push_str(p),
            Segment::Region(r) if r.column.is_some() => out.push_str(&r.body),
            Segment::Region(r) => {
                out.push_str(&r.raw_opener);
                out.push_str(&r.body);
                out.push_str(&r.raw_closer);
            }
        }
    }
    out
}

/// The headings of `template`'s prose as a reader sees them: none from a
/// region body or its markers, and a region inside a heading's line read
/// as its body. `Err` when the file does not parse.
pub(crate) fn prose_headings(template: &str) -> Result<Vec<Heading>, String> {
    let parsed = marker::parse(template).map_err(|e| e.to_string())?;
    // 0-based line ranges the regions on lines of their own take, markers
    // included.
    let regions: Vec<(usize, usize)> = parsed
        .segments
        .iter()
        .filter_map(|s| match s {
            Segment::Region(r) if r.column.is_none() => Some((r.line - 1, r.last_line())),
            _ => None,
        })
        .collect();
    Ok(project::headings(&shown(&parsed))
        .into_iter()
        .filter(|h| !regions.iter().any(|&(a, b)| (a..b).contains(&h.line)))
        .collect())
}

/// Whether a heading of `template`'s prose holds a region inside its line,
/// so the toc reads what a render of the template writes.
pub fn reads_regions(template: &str) -> bool {
    let Ok(parsed) = marker::parse(template) else {
        return false;
    };
    let lines: Vec<usize> = project::headings(template)
        .iter()
        .map(|h| h.line + 1)
        .collect();
    parsed
        .segments
        .iter()
        .any(|s| matches!(s, Segment::Region(r) if r.column.is_some() && lines.contains(&r.line)))
}

/// The levels a toc lists from its `min=` and `max=`, 1 to 6. The defaults
/// are 2 and 3; a bound given alone moves the other default out of its way,
/// so `min=4` lists level 4 only.
pub fn levels(min: Option<&str>, max: Option<&str>) -> Result<(usize, usize), String> {
    let level = |key: &str, v: &str| match v.parse::<usize>() {
        Ok(n) if (1..=6).contains(&n) && v.bytes().all(|b| b.is_ascii_digit()) => Ok(n),
        _ => Err(format!("{key}={v}: expected a heading level from 1 to 6")),
    };
    let min = min.map(|v| level("min", v)).transpose()?;
    let max = max.map(|v| level("max", v)).transpose()?;
    let (min, max) = match (min, max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, a.max(3)),
        (None, Some(b)) => (b.min(2), b),
        (None, None) => (2, 3),
    };
    if min > max {
        return Err(format!("min={min} is above max={max}"));
    }
    Ok((min, max))
}

/// The list and its snapshot for the headings of levels `min..=max` in
/// `template`'s prose.
pub fn toc(template: &str, min: usize, max: usize) -> Result<(String, Vec<u8>), String> {
    let prose = prose_headings(template).map_err(|e| format!("this file: {e}"))?;
    let mut slugger = Slugger::default();
    let mut text = String::new();
    let mut snapshot = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    for h in &prose {
        // Every heading takes its anchor, listed or not: GitHub counts
        // duplicates across the whole page.
        let anchor = slugger.slug(&h.text);
        if !(min..=max).contains(&h.level) {
            continue;
        }
        while open.last().is_some_and(|&l| l >= h.level) {
            open.pop();
        }
        text.push_str(&"  ".repeat(open.len()));
        text.push_str(&format!("- [{}](#{anchor})\n", link_text(&h.text)));
        open.push(h.level);
        snapshot.extend_from_slice(
            format!("{} {}\0#{anchor}\n", "#".repeat(h.level), h.text).as_bytes(),
        );
    }
    Ok((text, snapshot))
}

/// GitHub's anchors: github-slugger's rules over the heading's rendered
/// text, with `-1`, `-2` and so on for a slug already taken.
#[derive(Default)]
struct Slugger {
    taken: HashMap<String, usize>,
}

impl Slugger {
    fn slug(&mut self, heading: &str) -> String {
        let base = slug(&plain(heading));
        let mut result = base.clone();
        while self.taken.contains_key(&result) {
            let n = self.taken.get_mut(&base).expect("the base was taken first");
            *n += 1;
            result = format!("{base}-{n}");
        }
        self.taken.insert(result.clone(), 0);
        result
    }
}

/// Lowercased, every character but letters, digits, `-`, `_` and spaces
/// dropped, spaces turned into `-`.
fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|&c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// A heading's text as GitHub renders it, near enough for its anchor: code
/// spans keep their content, links and images their text, HTML tags go,
/// autolinks keep their address, backslash escapes and a few entities are
/// resolved, and `_` that opens or closes emphasis goes. `*` and other
/// punctuation the slug drops anyway.
fn plain(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '`' => {
                let run = chars[i..].iter().take_while(|&&x| x == '`').count();
                match code_span_end(&chars, i + run, run) {
                    Some(end) => {
                        let inner: String = chars[i + run..end].iter().collect();
                        let inner = match inner.strip_prefix(' ').and_then(|s| s.strip_suffix(' '))
                        {
                            Some(s) if !s.trim().is_empty() => s.to_string(),
                            _ => inner,
                        };
                        out.push_str(&inner);
                        i = end + run;
                    }
                    None => {
                        out.extend(&chars[i..i + run]);
                        i += run;
                    }
                }
            }
            '\\' if chars.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '!' if chars.get(i + 1) == Some(&'[') => i += 1,
            '[' => match link(&chars, i) {
                Some((label, end)) => {
                    out.push_str(&plain(&label));
                    i = end;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            },
            '<' => match chars[i + 1..].iter().position(|&x| x == '>') {
                Some(len) => {
                    let inner: String = chars[i + 1..i + 1 + len].iter().collect();
                    if inner.contains(':') && !inner.contains(char::is_whitespace) {
                        out.push_str(&inner);
                    }
                    i += len + 2;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            },
            '&' => {
                let rest: String = chars[i..chars.len().min(i + 6)].iter().collect();
                match [
                    ("&amp;", '&'),
                    ("&lt;", '<'),
                    ("&gt;", '>'),
                    ("&quot;", '"'),
                ]
                .iter()
                .find(|(e, _)| rest.starts_with(e))
                {
                    Some((e, ch)) => {
                        out.push(*ch);
                        i += e.len();
                    }
                    None => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            '_' => {
                let run = chars[i..].iter().take_while(|&&x| x == '_').count();
                let before = i > 0 && chars[i - 1].is_alphanumeric();
                let after = chars.get(i + run).is_some_and(|x| x.is_alphanumeric());
                if before == after {
                    out.extend(&chars[i..i + run]);
                }
                i += run;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Where a code span opened by a run of `run` backticks ends: the start of
/// the next run of exactly that length.
fn code_span_end(chars: &[char], from: usize, run: usize) -> Option<usize> {
    let mut j = from;
    while j < chars.len() {
        if chars[j] == '`' {
            let len = chars[j..].iter().take_while(|&&x| x == '`').count();
            if len == run {
                return Some(j);
            }
            j += len;
        } else {
            j += 1;
        }
    }
    None
}

/// A `[label](destination)` link starting at `chars[at]`: its label and the
/// index past its closing parenthesis.
fn link(chars: &[char], at: usize) -> Option<(String, usize)> {
    let mut depth = 0;
    let mut close = None;
    for (j, &c) in chars.iter().enumerate().skip(at) {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(j);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = close + 2 + chars[close + 2..].iter().position(|&c| c == ')')?;
    Some((chars[at + 1..close].iter().collect(), end + 1))
}

/// The heading text as a link's text: a link inside it keeps only its
/// label, since links do not nest.
fn link_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let image = i > 0 && chars[i - 1] == '!';
        match (chars[i], image) {
            ('[', false) => match link(&chars, i) {
                Some((label, end)) => {
                    out.push_str(&link_text(&label));
                    i = end;
                }
                None => {
                    out.push('[');
                    i += 1;
                }
            },
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_follow_github() {
        for (heading, want) in [
            ("Hello World", "hello-world"),
            ("What is v0 for?", "what-is-v0-for"),
            ("The `file` loader", "the-file-loader"),
            ("Why two sums?", "why-two-sums"),
            ("`run --dry-run`", "run---dry-run"),
            ("snake_case and __init__", "snake_case-and-init"),
            ("_emphasis_ and *strong*", "emphasis-and-strong"),
            ("A [link](https://example.com) here", "a-link-here"),
            ("An ![image](x.png) here", "an-image-here"),
            ("Tom &amp; Jerry", "tom--jerry"),
            ("<https://a.b/c>", "httpsabc"),
            ("<em>Tagged</em> text", "tagged-text"),
            ("Ünïcödé Straße", "ünïcödé-straße"),
            ("Emoji 🚀 launch", "emoji--launch"),
            ("a \\_ b", "a-_-b"),
            ("C++ & C#", "c--c"),
        ] {
            assert_eq!(slug(&plain(heading)), want, "{heading:?}");
        }
    }

    #[test]
    fn a_taken_slug_gets_a_number() {
        let mut s = Slugger::default();
        let got: Vec<String> = ["Usage", "Usage", "Usage-1", "Usage"]
            .iter()
            .map(|h| s.slug(h))
            .collect();
        assert_eq!(got, ["usage", "usage-1", "usage-1-1", "usage-2"]);
    }

    const TEMPLATE: &str = "# Title\n\n<!-- computed toc -->\n<!-- /computed -->\n\n## Install\n\n```sh\n## not a heading\n```\n\n### From source\n\n#### Deep\n\n<!-- computed file src=x.md -->\n## Inside a region\n<!-- /computed -->\n\n## Use [it](https://x)\n\n### Install\n\n## Install\n";

    #[test]
    fn a_nested_list_of_prose_headings() {
        let (text, snapshot) = toc(TEMPLATE, 2, 3).unwrap();
        assert_eq!(
            text,
            "- [Install](#install)\n  - [From source](#from-source)\n- [Use it](#use-it)\n  - [Install](#install-1)\n- [Install](#install-2)\n"
        );
        assert_eq!(
            String::from_utf8(snapshot).unwrap(),
            "## Install\0#install\n### From source\0#from-source\n## Use [it](https://x)\0#use-it\n### Install\0#install-1\n## Install\0#install-2\n"
        );
        let (text, _) = toc(TEMPLATE, 1, 6).unwrap();
        assert!(
            text.starts_with("- [Title](#title)\n  - [Install](#install)\n"),
            "{text}"
        );
        assert!(text.contains("      - [Deep](#deep)\n"), "{text}");
    }

    #[test]
    fn a_skipped_level_nests_under_the_nearest_open_heading() {
        let (text, _) = toc("## A\n#### B\n### C\n## D\n", 2, 4).unwrap();
        assert_eq!(text, "- [A](#a)\n  - [B](#b)\n  - [C](#c)\n- [D](#d)\n");
        let (text, _) = toc("### Deep first\n## Then\n", 2, 3).unwrap();
        assert_eq!(text, "- [Deep first](#deep-first)\n- [Then](#then)\n");
    }

    #[test]
    fn a_region_body_does_not_move_the_toc() {
        let before = toc(TEMPLATE, 2, 3).unwrap();
        let rendered = TEMPLATE.replace("## Inside a region\n", "# Many\n## More\n### Lines\n\n\n");
        assert_eq!(toc(&rendered, 2, 3).unwrap(), before);
    }

    #[test]
    fn crlf_headings_are_read() {
        let (text, _) = toc("## One\r\n\r\n## Two ##\r\n", 2, 3).unwrap();
        assert_eq!(text, "- [One](#one)\n- [Two](#two)\n");
    }
}
