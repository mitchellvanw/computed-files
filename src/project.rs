//! Projections: the part of a file a region depends on.
//!
//! A projection narrows a file to a slice of it, so a region that reads part
//! of a file is stale only when that part changes. Four kinds: `lines=A-B`,
//! `section=TEXT`, `anchor=NAME` and `key=a.b.c`. The `file` loader takes the
//! first three, `value` takes `key=`, and an exec `inputs=` entry takes any
//! of them as a `#` suffix on a literal path. Pure: bytes in, bytes out.

use std::path::Path;

use crate::marker;

/// The projection kinds, as attribute names and `inputs=` suffixes.
pub const KINDS: &[&str] = &["lines", "section", "anchor", "key"];

/// The projections the `file` loader takes; `key=` belongs to `value`.
pub const FILE_SLICES: &[&str] = &["lines", "section", "anchor"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    /// 1-based and inclusive; `last` is `None` for `A-`, to the end.
    Lines { first: usize, last: Option<usize> },
    /// A Markdown heading with this text, through the line before the next
    /// heading of the same or a higher level, heading line included.
    Section(String),
    /// The lines between `ANCHOR: NAME` and `ANCHOR_END: NAME`, mdBook's
    /// convention, the marker lines excluded.
    Anchor(String),
    /// A dotted path into a TOML, JSON or YAML file; a number indexes an array.
    Key(Vec<String>),
}

impl Projection {
    /// Parses one projection from its kind and value, as an attribute or an
    /// `inputs=` suffix spells them.
    pub fn parse(kind: &str, value: &str) -> Result<Projection, String> {
        match kind {
            "lines" => {
                let bad = || format!("lines={value}: expected A-B, A- or A, lines counted from 1");
                let (a, b) = value.split_once('-').unwrap_or((value, value));
                let first = number(a).ok_or_else(bad)?;
                let last = match b {
                    "" => None,
                    b => Some(number(b).ok_or_else(bad)?),
                };
                if first == 0 {
                    return Err(format!("lines={value}: lines are counted from 1"));
                }
                if last.is_some_and(|l| l < first) {
                    return Err(format!("lines={value}: the range ends before it starts"));
                }
                Ok(Projection::Lines { first, last })
            }
            "section" if value.trim().is_empty() => {
                Err("section=: expected the text of a heading".to_string())
            }
            "section" => Ok(Projection::Section(value.to_string())),
            "anchor" if value.is_empty() || !value.chars().all(anchor_char) => Err(format!(
                "anchor={value}: expected a name of letters, digits, `_` and `-`"
            )),
            "anchor" => Ok(Projection::Anchor(value.to_string())),
            "key" => {
                let path: Vec<String> = value.split('.').map(str::to_string).collect();
                if path.iter().any(String::is_empty) {
                    return Err(format!(
                        "key={value}: expected a dotted path such as package.version"
                    ));
                }
                Ok(Projection::Key(path))
            }
            other => Err(format!(
                "unknown projection {other}=: expected one of {}",
                KINDS.join(", ")
            )),
        }
    }

    /// `kind=value` with the value in one spelling, for snapshot entries.
    pub fn canonical(&self) -> String {
        match self {
            Projection::Lines { first, last } => match last {
                Some(last) => format!("lines={first}-{last}"),
                None => format!("lines={first}-"),
            },
            Projection::Section(text) => format!("section={text}"),
            Projection::Anchor(name) => format!("anchor={name}"),
            Projection::Key(path) => format!("key={}", path.join(".")),
        }
    }

    /// The slice of `content` this projection selects. `path` names the file
    /// for messages and, for `key=`, picks the format by its extension. The
    /// `key=` slice is the value in canonical JSON, keys sorted, so the
    /// file's formatting and every other key are left out. Finding nothing
    /// is an error.
    pub fn apply(&self, path: &Path, content: &[u8]) -> Result<Vec<u8>, String> {
        let text = || {
            std::str::from_utf8(content)
                .map_err(|_| format!("{}: {} is not UTF-8", self.canonical(), path.display()))
        };
        match self {
            Projection::Lines { first, last } => {
                let count = content.split_inclusive(|&b| b == b'\n').count();
                let last = last.unwrap_or(count);
                if *first > count || last > count {
                    return Err(format!(
                        "{}: the file has {count} line{}",
                        self.canonical(),
                        if count == 1 { "" } else { "s" }
                    ));
                }
                Ok(line_span(content, first - 1, last).to_vec())
            }
            Projection::Section(want) => {
                let text = text()?;
                let all = headings(text);
                let Some(k) = all.iter().position(|h| h.text == *want) else {
                    return Err(format!("{}: no heading reads {want:?}", self.canonical()));
                };
                let end = all[k + 1..]
                    .iter()
                    .find(|h| h.level <= all[k].level)
                    .map_or(usize::MAX, |h| h.line);
                Ok(line_span(text.as_bytes(), all[k].line, end).to_vec())
            }
            Projection::Anchor(name) => anchor(text()?, name)
                .map(String::into_bytes)
                .map_err(|e| format!("{}: {e}", self.canonical())),
            Projection::Key(key) => lookup(path, text()?, key)
                .map(|v| v.canonical().into_bytes())
                .map_err(|e| format!("{}: {e}", self.canonical())),
        }
    }
}

/// The one projection among an opener's attributes, from the kinds a
/// loader takes. More than one is an error: a slice of a slice is not
/// something any region has needed.
pub fn from_attrs(
    attrs: &[(String, String)],
    kinds: &[&str],
) -> Result<Option<Projection>, String> {
    let mut found = attrs.iter().filter(|(k, _)| kinds.contains(&k.as_str()));
    let Some((kind, value)) = found.next() else {
        return Ok(None);
    };
    if let Some((other, _)) = found.next() {
        return Err(format!(
            "{kind}= and {other}= both narrow the file; take one"
        ));
    }
    Projection::parse(kind, value).map(Some)
}

/// A run of ASCII digits as a number; `+5` and ` 5` are not numbers here.
fn number(s: &str) -> Option<usize> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn anchor_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Splits an `inputs=` entry at its projection: the first `#` followed by a
/// projection kind and `=`. Any other `#` is part of the path, as globset
/// reads it. A projection needs a literal path: a wildcard would project
/// every file it matched the same way, which no region means.
pub fn split_input(entry: &str) -> Result<(&str, Option<Projection>), String> {
    let at = KINDS
        .iter()
        .filter_map(|k| entry.find(&format!("#{k}=")))
        .min();
    let Some(at) = at else {
        return Ok((entry, None));
    };
    let (path, suffix) = (&entry[..at], &entry[at + 1..]);
    let (kind, value) = suffix.split_once('=').expect("found with a `=`");
    let projection = Projection::parse(kind, value)?;
    if path.contains(['*', '?', '[', '{', '\\']) {
        return Err(format!(
            "{entry}: a projection needs a literal path, not a wildcard"
        ));
    }
    if path.trim().is_empty() {
        return Err(format!("{entry}: a projection needs a path before the #"));
    }
    Ok((path, Some(projection)))
}

/// Lines `from..to` of `content`, 0-based, `to` exclusive and clamped,
/// each with its terminator.
fn line_span(content: &[u8], from: usize, to: usize) -> &[u8] {
    let mut start = content.len();
    let mut end = content.len();
    let mut offset = 0;
    for (i, line) in content.split_inclusive(|&b| b == b'\n').enumerate() {
        if i == from {
            start = offset;
        }
        if i == to {
            end = offset;
            break;
        }
        offset += line.len();
    }
    &content[start.min(end)..end]
}

/// The lines of `text` split at LF, without terminators, as the marker
/// parser counts them.
fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n')
        .map(|l| {
            l.strip_suffix('\n')
                .map_or(l, |l| l.strip_suffix('\r').unwrap_or(l))
        })
        .collect()
}

/// One ATX heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 0-based index of the heading's line.
    pub line: usize,
    /// 1 to 6.
    pub level: usize,
    /// The text as written, without the `#` runs and surrounding whitespace.
    pub text: String,
}

/// The ATX headings of a Markdown text in order, leaving out lines inside
/// fenced code blocks, as the marker parser reads fences, and a YAML front
/// matter block at the top. Setext headings are not read.
pub fn headings(text: &str) -> Vec<Heading> {
    let lines = lines(text);
    let fenced = marker::fenced(text);
    let skip = front_matter(&lines);
    lines
        .iter()
        .enumerate()
        .skip(skip)
        .filter(|&(i, _)| !fenced.get(i).copied().unwrap_or(false))
        .filter_map(|(i, l)| {
            atx(l).map(|(level, text)| Heading {
                line: i,
                level,
                text: text.to_string(),
            })
        })
        .collect()
}

/// The number of lines a `---` front matter block at the top takes, closer
/// included; 0 when there is none.
fn front_matter(lines: &[&str]) -> usize {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return 0;
    }
    lines[1..]
        .iter()
        .position(|l| matches!(l.trim_end(), "---" | "..."))
        .map_or(0, |j| j + 2)
}

/// A CommonMark ATX heading: up to three spaces, one to six `#`, then a
/// space, a tab or the end of the line. The optional closing run of `#` is
/// dropped when a space precedes it.
fn atx(line: &str) -> Option<(usize, &str)> {
    let stripped = line.trim_start_matches(' ');
    if line.len() - stripped.len() > 3 {
        return None;
    }
    let level = stripped.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &stripped[level..];
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let text = rest.trim_matches([' ', '\t']);
    let open = text.trim_end_matches('#');
    let text = if open.is_empty() {
        open
    } else if open.ends_with([' ', '\t']) {
        open.trim_end_matches([' ', '\t'])
    } else {
        text
    };
    Some((level, text))
}

/// The lines between `ANCHOR: name` and `ANCHOR_END: name`. Lines that mark
/// other anchors inside are left out, as mdBook leaves them out.
fn anchor(text: &str, name: &str) -> Result<String, String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(start) = lines
        .iter()
        .position(|l| anchor_name(l, "ANCHOR:") == Some(name))
    else {
        return Err(format!("no line holds ANCHOR: {name}"));
    };
    let Some(len) = lines[start + 1..]
        .iter()
        .position(|l| anchor_name(l, "ANCHOR_END:") == Some(name))
    else {
        return Err(format!(
            "ANCHOR: {name} on line {} has no ANCHOR_END: {name} below it",
            start + 1
        ));
    };
    Ok(lines[start + 1..start + 1 + len]
        .iter()
        .filter(|l| anchor_name(l, "ANCHOR:").is_none() && anchor_name(l, "ANCHOR_END:").is_none())
        .copied()
        .collect())
}

/// The anchor name after `tag` on a line, if the line holds the tag.
fn anchor_name<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let rest = line[line.find(tag)? + tag.len()..].trim_start();
    let end = rest.find(|c| !anchor_char(c)).unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// A value read from a TOML, JSON or YAML file, in one shape for all three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    /// A number, boolean or date, in its format's canonical spelling.
    Literal(String),
    Null,
    Array(Vec<Value>),
    Table(Vec<(String, Value)>),
}

impl Value {
    /// The text of a scalar: a string without quotes, a literal as spelled,
    /// `null`. `None` for an array or a table.
    pub fn scalar(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Literal(s) => Some(s),
            Value::Null => Some("null"),
            Value::Array(_) | Value::Table(_) => None,
        }
    }

    /// What kind of value this is, for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Str(_) | Value::Literal(_) | Value::Null => "a scalar",
            Value::Array(_) => "an array",
            Value::Table(_) => "a table",
        }
    }

    /// Compact JSON with table keys sorted: the same value in any format or
    /// layout spells the same. Literals are written as their format spells
    /// them.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Str(s) => out.push_str(&serde_json::Value::from(s.as_str()).to_string()),
            Value::Literal(s) => out.push_str(s),
            Value::Null => out.push_str("null"),
            Value::Array(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Value::Table(entries) => {
                let mut sorted: Vec<&(String, Value)> = entries.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                out.push('{');
                for (i, (k, v)) in sorted.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::Value::from(k.as_str()).to_string());
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// The value at `key` in a data file, its format chosen by the extension
/// of `path`: `.toml`, `.json`, `.yaml` or `.yml`.
pub fn lookup(path: &Path, text: &str, key: &[String]) -> Result<Value, String> {
    let root = parse_data(path, text)?;
    let mut at = &root;
    for (i, seg) in key.iter().enumerate() {
        let here = if i == 0 {
            "the top level".to_string()
        } else {
            key[..i].join(".")
        };
        at = match at {
            Value::Table(entries) => entries
                .iter()
                .find(|(k, _)| k == seg)
                .map(|(_, v)| v)
                .ok_or_else(|| format!("{here} has no key {seg:?}"))?,
            Value::Array(items) => {
                let n = number(seg)
                    .ok_or_else(|| format!("{here} is an array; expected an index, not {seg:?}"))?;
                items.get(n).ok_or_else(|| {
                    format!(
                        "{here} has {} item{}; there is no index {n}",
                        items.len(),
                        if items.len() == 1 { "" } else { "s" }
                    )
                })?
            }
            scalar => return Err(format!("{here} is {}, not a table", scalar.kind())),
        };
    }
    Ok(at.clone())
}

fn parse_data(path: &Path, text: &str) -> Result<Value, String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    let file = path.display();
    match ext.as_deref() {
        Some("toml") => text
            .parse::<toml::Table>()
            .map(|t| from_toml(toml::Value::Table(t)))
            .map_err(|e| format!("{file}: {}", e.to_string().trim_end())),
        Some("json") => serde_json::from_str::<serde_json::Value>(text)
            .map(from_json)
            .map_err(|e| format!("{file}: {e}")),
        Some("yaml" | "yml") => {
            let docs =
                yaml_rust2::YamlLoader::load_from_str(text).map_err(|e| format!("{file}: {e}"))?;
            match docs.into_iter().next() {
                Some(doc) => from_yaml(doc).map_err(|e| format!("{file}: {e}")),
                None => Ok(Value::Null),
            }
        }
        _ => Err(format!(
            "{file}: key= reads a .toml, .json, .yaml or .yml file"
        )),
    }
}

fn from_toml(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::Str(s),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(from_toml).collect()),
        toml::Value::Table(t) => {
            Value::Table(t.into_iter().map(|(k, v)| (k, from_toml(v))).collect())
        }
        literal => Value::Literal(literal.to_string()),
    }
}

fn from_json(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::String(s) => Value::Str(s),
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Array(items) => Value::Array(items.into_iter().map(from_json).collect()),
        serde_json::Value::Object(o) => {
            Value::Table(o.into_iter().map(|(k, v)| (k, from_json(v))).collect())
        }
        literal => Value::Literal(literal.to_string()),
    }
}

fn from_yaml(v: yaml_rust2::Yaml) -> Result<Value, String> {
    use yaml_rust2::Yaml;
    Ok(match v {
        Yaml::String(s) => Value::Str(s),
        Yaml::Real(s) => Value::Literal(s),
        Yaml::Integer(n) => Value::Literal(n.to_string()),
        Yaml::Boolean(b) => Value::Literal(b.to_string()),
        Yaml::Null => Value::Null,
        Yaml::Array(items) => {
            Value::Array(items.into_iter().map(from_yaml).collect::<Result<_, _>>()?)
        }
        Yaml::Hash(h) => Value::Table(
            h.into_iter()
                .map(|(k, v)| {
                    let key = match from_yaml(k)? {
                        Value::Str(s) | Value::Literal(s) => s,
                        other => return Err(format!("a key is {}, not a scalar", other.kind())),
                    };
                    Ok((key, from_yaml(v)?))
                })
                .collect::<Result<_, _>>()?,
        ),
        Yaml::Alias(_) | Yaml::BadValue => return Err("an alias is not supported".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(kind: &str, value: &str, path: &str, content: &str) -> Result<String, String> {
        Projection::parse(kind, value)?
            .apply(Path::new(path), content.as_bytes())
            .map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn parse_and_canonical_round_trip() {
        for (kind, value, canonical) in [
            ("lines", "40-90", "lines=40-90"),
            ("lines", "7-", "lines=7-"),
            ("lines", "3", "lines=3-3"),
            ("section", "Getting started", "section=Getting started"),
            ("anchor", "setup-2", "anchor=setup-2"),
            ("key", "package.version", "key=package.version"),
            ("key", "a.0.b", "key=a.0.b"),
        ] {
            assert_eq!(
                Projection::parse(kind, value).unwrap().canonical(),
                canonical
            );
        }
    }

    #[test]
    fn malformed_projections_are_errors() {
        for (kind, value, needle) in [
            ("lines", "0-3", "from 1"),
            ("lines", "5-2", "before it starts"),
            ("lines", "a-b", "expected A-B"),
            ("lines", "+1-2", "expected A-B"),
            ("lines", "", "expected A-B"),
            ("section", " ", "heading"),
            ("anchor", "a b", "name"),
            ("anchor", "", "name"),
            ("key", "a..b", "dotted path"),
            ("key", "", "dotted path"),
            ("bytes", "1-2", "unknown projection"),
        ] {
            let e = Projection::parse(kind, value).unwrap_err();
            assert!(e.contains(needle), "{kind}={value}: {e}");
        }
    }

    #[test]
    fn lines_slices_inclusive_and_keeps_terminators() {
        let text = "one\ntwo\r\nthree\nfour";
        assert_eq!(apply("lines", "2-3", "f", text).unwrap(), "two\r\nthree\n");
        assert_eq!(apply("lines", "3-", "f", text).unwrap(), "three\nfour");
        assert_eq!(apply("lines", "4", "f", text).unwrap(), "four");
        let e = apply("lines", "3-5", "f", text).unwrap_err();
        assert_eq!(e, "lines=3-5: the file has 4 lines");
        assert!(
            apply("lines", "1-", "f", "")
                .unwrap_err()
                .contains("0 lines")
        );
    }

    const DOC: &str = "---\ntitle: x\n# not a heading\n---\n# Title\n\nintro\n\n## Install\n\nsteps\n\n```sh\n# a comment, not a heading\n```\n\n### Details\n\nmore\n\n## Use ##\n\nuse it\n";

    #[test]
    fn headings_skip_fences_and_front_matter() {
        let all = headings(DOC);
        let got: Vec<(usize, usize, &str)> = all
            .iter()
            .map(|h| (h.line, h.level, h.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (4, 1, "Title"),
                (8, 2, "Install"),
                (16, 3, "Details"),
                (20, 2, "Use")
            ]
        );
    }

    #[test]
    fn atx_follows_commonmark() {
        for (line, want) in [
            ("# a", Some((1, "a"))),
            ("   ## b ##  ", Some((2, "b"))),
            ("#", Some((1, ""))),
            ("### ###", Some((3, ""))),
            ("## c#", Some((2, "c#"))),
            ("## c \\#", Some((2, "c \\#"))),
            ("#\tt", Some((1, "t"))),
            ("#hashtag", None),
            ("####### seven", None),
            ("    # indented code", None),
            ("\t# tab", None),
        ] {
            assert_eq!(atx(line), want, "{line:?}");
        }
    }

    #[test]
    fn section_runs_to_the_next_heading_of_the_same_or_higher_level() {
        assert_eq!(
            apply("section", "Install", "f.md", DOC).unwrap(),
            "## Install\n\nsteps\n\n```sh\n# a comment, not a heading\n```\n\n### Details\n\nmore\n\n"
        );
        assert_eq!(
            apply("section", "Details", "f.md", DOC).unwrap(),
            "### Details\n\nmore\n\n"
        );
        assert_eq!(
            apply("section", "Use", "f.md", DOC).unwrap(),
            "## Use ##\n\nuse it\n"
        );
        assert_eq!(
            apply("section", "Title", "f.md", DOC).unwrap(),
            &DOC[DOC.find("# Title").unwrap()..]
        );
        let e = apply("section", "a comment, not a heading", "f.md", DOC).unwrap_err();
        assert!(e.contains("no heading reads"), "{e}");
        let e = apply("section", "Missing", "f.md", DOC).unwrap_err();
        assert!(e.starts_with("section=Missing: "), "{e}");
    }

    #[test]
    fn section_reads_crlf_text() {
        let text = "# A\r\none\r\n# B\r\ntwo\r\n";
        assert_eq!(
            apply("section", "A", "f.md", text).unwrap(),
            "# A\r\none\r\n"
        );
    }

    #[test]
    fn a_duplicate_heading_takes_the_first() {
        let text = "## A\n1\n## A\n2\n";
        assert_eq!(apply("section", "A", "f.md", text).unwrap(), "## A\n1\n");
    }

    #[test]
    fn anchor_takes_the_lines_between_its_markers() {
        let text = "fn main() {\n    // ANCHOR: body\n    let x = 1;\n    // ANCHOR: inner\n    let y = 2;\n    // ANCHOR_END: inner\n    // ANCHOR_END: body\n}\n";
        assert_eq!(
            apply("anchor", "body", "f.rs", text).unwrap(),
            "    let x = 1;\n    let y = 2;\n"
        );
        assert_eq!(
            apply("anchor", "inner", "f.rs", text).unwrap(),
            "    let y = 2;\n"
        );
        let e = apply("anchor", "bod", "f.rs", text).unwrap_err();
        assert!(e.contains("no line holds ANCHOR: bod"), "{e}");
        let e = apply("anchor", "open", "f.rs", "// ANCHOR: open\nx\n").unwrap_err();
        assert!(e.contains("line 1 has no ANCHOR_END: open"), "{e}");
        let e = apply("anchor", "b", "f.rs", "// ANCHOR_END: b\n// ANCHOR: b\n").unwrap_err();
        assert!(
            e.contains("has no ANCHOR_END"),
            "an end above the start is unbalanced: {e}"
        );
        assert_eq!(
            apply("anchor", "e", "f.rs", "# ANCHOR: e\n# ANCHOR_END: e\n").unwrap(),
            ""
        );
    }

    const TOML: &str = "[package]\nname = \"computed\"\nversion = \"0.2.0\"\nedition = 2024\nratio = 1.0\n\n[[bin]]\nname = \"one\"\n\n[[bin]]\nname = \"two\"\n";

    #[test]
    fn key_reads_toml_json_and_yaml() {
        let key = |k: &str, path: &str, text: &str| {
            let key: Vec<String> = k.split('.').map(str::to_string).collect();
            lookup(Path::new(path), text, &key)
        };
        assert_eq!(
            key("package.version", "Cargo.toml", TOML).unwrap().scalar(),
            Some("0.2.0")
        );
        assert_eq!(
            key("package.edition", "Cargo.toml", TOML).unwrap().scalar(),
            Some("2024")
        );
        assert_eq!(
            key("package.ratio", "Cargo.toml", TOML).unwrap().scalar(),
            Some("1.0")
        );
        assert_eq!(
            key("bin.1.name", "Cargo.toml", TOML).unwrap().scalar(),
            Some("two")
        );
        let json = r#"{"name": "x", "version": "1.2.3", "n": 1.50, "ok": true, "none": null, "list": [{"a": 1}]}"#;
        assert_eq!(
            key("version", "package.json", json).unwrap().scalar(),
            Some("1.2.3")
        );
        assert_eq!(
            key("n", "package.json", json).unwrap().scalar(),
            Some("1.5")
        );
        assert_eq!(
            key("ok", "package.json", json).unwrap().scalar(),
            Some("true")
        );
        assert_eq!(
            key("none", "package.json", json).unwrap().scalar(),
            Some("null")
        );
        assert_eq!(
            key("list.0.a", "package.json", json).unwrap().scalar(),
            Some("1")
        );
        let yaml = "on:\n  push:\n    branches: [main]\nversion: 1.10\nname: ci\n";
        assert_eq!(
            key("on.push.branches.0", "ci.yml", yaml).unwrap().scalar(),
            Some("main")
        );
        assert_eq!(
            key("version", "ci.yaml", yaml).unwrap().scalar(),
            Some("1.10")
        );
        assert_eq!(key("name", "ci.YML", yaml).unwrap().scalar(), Some("ci"));
    }

    #[test]
    fn key_errors_name_where_the_path_stopped() {
        for (k, path, text, needle) in [
            (
                "package.nope",
                "Cargo.toml",
                TOML,
                "package has no key \"nope\"",
            ),
            ("nope", "Cargo.toml", TOML, "the top level has no key"),
            (
                "bin.5",
                "Cargo.toml",
                TOML,
                "bin has 2 items; there is no index 5",
            ),
            ("bin.first", "Cargo.toml", TOML, "bin is an array"),
            (
                "package.name.x",
                "Cargo.toml",
                TOML,
                "package.name is a scalar",
            ),
            ("a", "notes.txt", "a = 1", "reads a .toml"),
            ("a", "bad.toml", "a = ", "bad.toml"),
            ("a", "bad.json", "{", "bad.json"),
        ] {
            let e = apply("key", k, path, text).unwrap_err();
            assert!(e.contains(needle), "{k} in {path}: {e}");
            assert!(e.starts_with(&format!("key={k}: ")), "{e}");
        }
    }

    #[test]
    fn a_key_projection_is_canonical_json_whatever_the_layout() {
        let a = apply("key", "package", "Cargo.toml", TOML).unwrap();
        assert_eq!(
            a,
            r#"{"edition":2024,"name":"computed","ratio":1.0,"version":"0.2.0"}"#
        );
        let reordered = "[package]\nversion = \"0.2.0\"\nratio = 1.0\n# a comment\nedition = 2024\nname = \"computed\"\n";
        assert_eq!(apply("key", "package", "Cargo.toml", reordered).unwrap(), a);
        let json = r#"{"package": {"version": "0.2.0", "name": "computed", "edition": 2024, "ratio": 1.0}}"#;
        assert_eq!(apply("key", "package", "x.json", json).unwrap(), a);
        assert_eq!(
            apply("key", "package.name", "Cargo.toml", TOML).unwrap(),
            "\"computed\""
        );
        assert_eq!(
            apply("key", "bin", "Cargo.toml", TOML).unwrap(),
            r#"[{"name":"one"},{"name":"two"}]"#
        );
    }

    #[test]
    fn non_utf8_content_fails_every_projection_but_lines() {
        let bytes = b"a\n\xff\n";
        let p = Projection::parse("lines", "2").unwrap();
        assert_eq!(p.apply(Path::new("f"), bytes).unwrap(), b"\xff\n");
        let p = Projection::parse("section", "a").unwrap();
        assert!(
            p.apply(Path::new("f"), bytes)
                .unwrap_err()
                .contains("not UTF-8")
        );
    }

    #[test]
    fn split_input_finds_the_projection_suffix() {
        assert_eq!(split_input("docs/*.md").unwrap(), ("docs/*.md", None));
        assert_eq!(split_input("notes/#1.md").unwrap(), ("notes/#1.md", None));
        assert_eq!(
            split_input("Cargo.toml#key=package.version").unwrap(),
            (
                "Cargo.toml",
                Some(Projection::Key(vec!["package".into(), "version".into()]))
            )
        );
        assert_eq!(
            split_input("README.md#section=C# and #lines=1").unwrap(),
            (
                "README.md",
                Some(Projection::Section("C# and #lines=1".into()))
            )
        );
        assert_eq!(
            split_input("src/cli.rs#lines=40-90").unwrap().1,
            Some(Projection::Lines {
                first: 40,
                last: Some(90)
            })
        );
        for (entry, needle) in [
            ("src/*.rs#lines=1-2", "literal path"),
            ("#key=a", "path before"),
            ("a.toml#key=", "dotted path"),
            ("a.md#lines=2-1", "before it starts"),
        ] {
            let e = split_input(entry).unwrap_err();
            assert!(e.contains(needle), "{entry}: {e}");
        }
    }
}
