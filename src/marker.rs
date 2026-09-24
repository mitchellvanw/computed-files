//! The marker grammar: parse a file into prose and regions, and serialise it back.
//!
//! A marker is a whole line: optional indentation, an HTML comment whose first
//! token is `computed` (opener) or `/computed` (closer), optional trailing
//! whitespace. Markers inside CommonMark fenced code blocks are prose. The
//! raw lines are kept so a fresh region can be reproduced byte for byte.

use std::fmt;

use crate::table::TableFrom;

/// A parsed template: prose and regions in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Segment {
    /// Text outside any region, exactly as it sits in the file.
    Prose(String),
    Region(Region),
}

/// The span between an opener and a closer.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// 1-based line number of the opener.
    pub line: usize,
    /// Leading whitespace of the opener line, preserved on write.
    pub indent: String,
    /// The opener line as it sits in the file, terminator included.
    pub raw_opener: String,
    /// The closer line as it sits in the file, terminator included when present.
    pub raw_closer: String,
    /// The lines strictly between the markers, each with its terminator.
    pub body: String,
    /// The closer's sums; `None` means the region is unrendered.
    pub sums: Option<Sums>,
    pub opener: Opener,
}

/// The two sums a rendered closer carries, as 64 lowercase hex characters each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sums {
    pub input: String,
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sink {
    Raw,
    Fence,
    /// `as=table`, with what the text is written in.
    Table(TableFrom),
}

impl Sink {
    fn parse(s: &str) -> Option<Sink> {
        match s {
            "raw" => Some(Sink::Raw),
            "fence" => Some(Sink::Fence),
            "table" => Some(Sink::Table(TableFrom::Delimited(b','))),
            _ => None,
        }
    }
}

/// What `check` makes of a region that is only stale: drift that fails, or,
/// with `on-stale=warn`, a line that does not raise the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnStale {
    Fail,
    Warn,
}

/// The parsed opener. `attrs` holds the loader's own attributes in the order
/// written; the common attributes `name=`, `as=`, `lang=`, `on-stale=` and
/// `max-lines=` are lifted out, and so are `delim=` and `from=`, which shape
/// `as=table` into `sink`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opener {
    pub loader: String,
    pub flags: Vec<String>,
    pub attrs: Vec<(String, String)>,
    pub name: Option<String>,
    pub sink: Sink,
    pub lang: String,
    pub on_stale: OnStale,
    /// The most lines of loader text the sink shapes; the rest is one note.
    pub max_lines: Option<usize>,
    /// The canonical form of the `use` opener this one was expanded from,
    /// which is what the file shows; `None` for an opener as written.
    written: Option<String>,
    /// Every token as written, in order, for the canonical form.
    tokens: Vec<Token>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Bare(String),
    Attr(String, String),
}

impl Opener {
    /// The single-space form without suffix or indentation. Hashed into the
    /// input sum and written into the rendered file.
    pub fn canonical(&self) -> String {
        let mut out = String::from("<!-- computed");
        for t in &self.tokens {
            out.push(' ');
            match t {
                Token::Bare(w) => out.push_str(w),
                Token::Attr(k, v) => {
                    out.push_str(k);
                    out.push('=');
                    out.push_str(&quote(v));
                }
            }
        }
        out.push_str(" -->");
        out
    }

    /// One attribute value by key.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    /// The common attributes as written, in order, with the table sink's
    /// `delim=` and `from=` unless the loader owns an attribute of that name.
    pub fn common_attrs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.tokens.iter().filter_map(|t| match t {
            Token::Attr(k, v)
                if is_common(k) || (SINK_ATTRS.contains(&k.as_str()) && self.attr(k).is_none()) =>
            {
                Some((k.as_str(), v.as_str()))
            }
            _ => None,
        })
    }

    /// This opener standing in for the `use` opener `written`: the file
    /// keeps showing `written`, and everything else reads this one.
    pub fn expanded_from(mut self, written: &Opener) -> Opener {
        self.written = Some(written.canonical());
        self
    }
}

/// The suffix the tool writes after the attributes of a rendered opener.
pub const OPENER_SUFFIX: &str = "| do not edit; run computed";

/// The rendered opener line: canonical form with the suffix, without indent.
pub fn rendered_opener(opener: &Opener) -> String {
    let c = opener.written.clone().unwrap_or_else(|| opener.canonical());
    let stem = c
        .strip_suffix(" -->")
        .expect("canonical opener ends with -->");
    format!("{stem} {OPENER_SUFFIX} -->")
}

/// The closer line for the given sums, without indent.
pub fn rendered_closer(sums: Option<&Sums>) -> String {
    match sums {
        Some(s) => format!("<!-- /computed in={} out={} -->", s.input, s.output),
        None => "<!-- /computed -->".to_string(),
    }
}

/// A value as the canonical form writes it: double-quoted when it is empty
/// or holds whitespace, `>` or `"`.
pub fn quote(v: &str) -> String {
    let needs = v.is_empty()
        || v.chars()
            .any(|c| c == ' ' || c == '\t' || c == '>' || c == '"');
    if !needs {
        return v.to_string();
    }
    let mut out = String::from("\"");
    for c in v.chars() {
        if c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

fn error(line: usize, message: impl Into<String>) -> ParseError {
    ParseError {
        line,
        message: message.into(),
    }
}

/// The attribute set each loader owns. Anything else is an unknown attribute.
struct LoaderGrammar {
    name: &'static str,
    attrs: &'static [&'static str],
    flags: &'static [&'static str],
    sink: Sink,
}

const GRAMMAR: &[LoaderGrammar] = &[
    LoaderGrammar {
        name: "tree",
        attrs: &["src", "depth"],
        flags: &["all", "dirs"],
        sink: Sink::Fence,
    },
    LoaderGrammar {
        name: "exec",
        attrs: &["cmd", "inputs", "timeout"],
        flags: &["volatile"],
        sink: Sink::Raw,
    },
    LoaderGrammar {
        name: "file",
        attrs: &["src", "lines", "section", "anchor"],
        flags: &[],
        sink: Sink::Raw,
    },
    LoaderGrammar {
        name: "value",
        attrs: &["src", "key"],
        flags: &[],
        sink: Sink::Raw,
    },
    LoaderGrammar {
        name: "index",
        attrs: &["src", "title"],
        flags: &[],
        sink: Sink::Raw,
    },
    LoaderGrammar {
        name: "toc",
        attrs: &["min", "max"],
        flags: &[],
        sink: Sink::Raw,
    },
    LoaderGrammar {
        name: "use",
        attrs: &["recipe"],
        flags: &[],
        sink: Sink::Raw,
    },
];

const COMMON_ATTRS: &[&str] = &["name", "as", "lang", "on-stale", "max-lines"];

/// The attributes that shape `as=table`, which any loader takes with it.
pub const SINK_ATTRS: &[&str] = &["delim", "from"];

/// Whether `key` is an attribute every loader takes.
pub fn is_common(key: &str) -> bool {
    COMMON_ATTRS.contains(&key)
}

/// One physical line of the file with its terminator.
struct Line<'a> {
    number: usize,
    /// The whole line, terminator included.
    raw: &'a str,
    /// The line without terminator.
    text: &'a str,
}

fn lines(text: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let raw = &text[start..=i];
            let t = raw.strip_suffix('\n').unwrap();
            let t = t.strip_suffix('\r').unwrap_or(t);
            out.push(Line {
                number: out.len() + 1,
                raw,
                text: t,
            });
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        let raw = &text[start..];
        out.push(Line {
            number: out.len() + 1,
            raw,
            text: raw,
        });
    }
    out
}

enum Kind<'a> {
    Prose,
    /// The content between `<!--` and `-->`, trimmed, plus the indent.
    Opener {
        indent: &'a str,
        content: &'a str,
    },
    Closer {
        indent: &'a str,
        content: &'a str,
    },
}

fn classify<'a>(line: &Line<'a>) -> Result<Kind<'a>, ParseError> {
    let text = line.text;
    let trimmed_start = text.trim_start_matches([' ', '\t']);
    let indent = &text[..text.len() - trimmed_start.len()];
    let Some(after) = trimmed_start.strip_prefix("<!--") else {
        return Ok(Kind::Prose);
    };
    if !after.starts_with([' ', '\t']) {
        return Ok(Kind::Prose);
    }
    let inner = after.trim_start_matches([' ', '\t']);
    let word_end = inner.find([' ', '\t']).unwrap_or(inner.len());
    // `<!-- /computed-->` closes as surely as `<!-- /computed -->`.
    let word = &inner[..word_end];
    let word = word.strip_suffix("-->").unwrap_or(word);
    if word != "computed" && word != "/computed" {
        return Ok(Kind::Prose);
    }
    let body = trimmed_start.trim_end_matches([' ', '\t']);
    let Some(content) = body.strip_suffix("-->") else {
        return Err(error(
            line.number,
            "unterminated marker: the line does not end with -->",
        ));
    };
    let content = content
        .strip_prefix("<!--")
        .unwrap()
        .trim_matches([' ', '\t']);
    let content = content[word.len()..].trim_start_matches([' ', '\t']);
    if word == "computed" {
        Ok(Kind::Opener { indent, content })
    } else {
        Ok(Kind::Closer { indent, content })
    }
}

/// A fence run at the start of a line: its character and length, with the
/// info string, when the line can open or close a CommonMark fence.
fn fence_run(text: &str) -> Option<(char, usize, &str)> {
    let stripped = text.trim_start_matches(' ');
    if text.len() - stripped.len() > 3 {
        return None;
    }
    let c = stripped.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = stripped.chars().take_while(|&x| x == c).count();
    (run >= 3).then_some((c, run, &stripped[run..]))
}

/// Whether `text` opens a fence it never closes. Such text in a `raw` body
/// would let a later fence in the file's prose swallow the closer.
pub fn has_unclosed_fence(text: &str) -> bool {
    let lines = lines(text);
    let fenced = fenced_lines(&lines);
    lines.iter().zip(&fenced).any(|(l, &f)| {
        !f && matches!(fence_run(l.text), Some((c, _, info)) if !(c == '`' && info.contains('`')))
    })
}

/// Which lines sit inside a fenced code block, fence lines included. A fence
/// counts only when a matching closer follows; an unclosed fence is prose.
fn fenced_lines(lines: &[Line<'_>]) -> Vec<bool> {
    let mut fenced = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let Some((c, run, info)) = fence_run(lines[i].text) else {
            i += 1;
            continue;
        };
        if c == '`' && info.contains('`') {
            i += 1;
            continue;
        }
        let closer = (i + 1..lines.len()).find(|&j| {
            matches!(fence_run(lines[j].text), Some((cc, r, rest)) if cc == c && r >= run && rest.trim().is_empty())
        });
        match closer {
            Some(j) => {
                fenced[i..=j].iter_mut().for_each(|f| *f = true);
                i = j + 1;
            }
            None => i += 1,
        }
    }
    fenced
}

/// Which lines of `text`, split at LF, sit inside a fenced code block as
/// the parser reads it, fence lines included.
pub fn fenced(text: &str) -> Vec<bool> {
    fenced_lines(&lines(text))
}

/// Whether a line, on its own, would parse as an opener or a closer.
pub fn is_marker(text: &str) -> bool {
    let line = Line {
        number: 1,
        raw: text,
        text,
    };
    !matches!(classify(&line), Ok(Kind::Prose))
}

/// The first line of `text` outside a fenced code block that would parse
/// as a marker. Such a line in a `raw` body would open or close a region.
pub fn unfenced_marker_line(text: &str) -> Option<&str> {
    let lines = lines(text);
    let fenced = fenced_lines(&lines);
    lines
        .iter()
        .zip(&fenced)
        .find(|(l, f)| !**f && is_marker(l.text))
        .map(|(l, _)| l.text)
}

/// The 1-based lines of fences in `text` that open a block no later fence
/// closes. The parser reads them as prose; a fence written below one can
/// close it and swallow what lies between.
pub fn unclosed_fences(text: &str) -> Vec<usize> {
    let lines = lines(text);
    let fenced = fenced_lines(&lines);
    lines
        .iter()
        .zip(&fenced)
        .filter(|(l, f)| {
            !**f && matches!(fence_run(l.text), Some((c, _, info)) if !(c == '`' && info.contains('`')))
        })
        .map(|(l, _)| l.number)
        .collect()
}

/// Whether any line of `text` would parse as a marker, fenced or not.
pub fn has_marker(text: &str) -> bool {
    lines(text).iter().any(|l| is_marker(l.text))
}

/// `bytes` with the sums taken out of every closer line, so a snapshot of
/// another template moves with its prose and bodies and not with the sums
/// the tool writes into it. Lines that are not UTF-8 are kept as they are.
pub fn strip_sums(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    const NEEDLE: &[u8] = b"/computed";
    if !bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
        return std::borrow::Cow::Borrowed(bytes);
    }
    let mut out = Vec::with_capacity(bytes.len());
    for raw in bytes.split_inclusive(|&b| b == b'\n') {
        let (body, term) = match raw.strip_suffix(b"\r\n") {
            Some(b) => (b, &b"\r\n"[..]),
            None => match raw.strip_suffix(b"\n") {
                Some(b) => (b, &b"\n"[..]),
                None => (raw, &b""[..]),
            },
        };
        let line = Line {
            number: 1,
            raw: "",
            text: std::str::from_utf8(body).unwrap_or(""),
        };
        match classify(&line) {
            Ok(Kind::Closer { indent, content }) if !content.is_empty() => {
                out.extend_from_slice(indent.as_bytes());
                out.extend_from_slice(rendered_closer(None).as_bytes());
                out.extend_from_slice(term);
            }
            _ => out.extend_from_slice(raw),
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Parses a file into prose and regions. Every grammar error is tier 2.
pub fn parse(text: &str) -> Result<File, ParseError> {
    let lines = lines(text);
    let mut segments = Vec::new();
    let mut prose = String::new();
    let fenced = fenced_lines(&lines);
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if fenced[i] {
            prose.push_str(line.raw);
            i += 1;
            continue;
        }
        match classify(line)? {
            Kind::Prose => {
                prose.push_str(line.raw);
                i += 1;
            }
            Kind::Closer { .. } => return Err(error(line.number, "closer without opener")),
            Kind::Opener { indent, content } => {
                let opener = parse_opener(line.number, content)?;
                if let Some(name) = &opener.name {
                    if names.contains(name) {
                        return Err(error(line.number, format!("duplicate name {name:?}")));
                    }
                    names.push(name.clone());
                }
                if !prose.is_empty() {
                    segments.push(Segment::Prose(std::mem::take(&mut prose)));
                }
                let mut body = String::new();
                let mut j = i + 1;
                let closer = loop {
                    let Some(l) = lines.get(j) else {
                        return Err(error(line.number, "opener without closer"));
                    };
                    if fenced[j] {
                        body.push_str(l.raw);
                        j += 1;
                        continue;
                    }
                    match classify(l)? {
                        Kind::Prose => body.push_str(l.raw),
                        Kind::Opener { .. } => {
                            return Err(error(
                                l.number,
                                "opener inside a body: nesting is not supported",
                            ));
                        }
                        Kind::Closer { content, .. } => {
                            break (l, parse_closer(l.number, content)?);
                        }
                    }
                    j += 1;
                };
                segments.push(Segment::Region(Region {
                    line: line.number,
                    indent: indent.to_string(),
                    raw_opener: line.raw.to_string(),
                    raw_closer: closer.0.raw.to_string(),
                    body,
                    sums: closer.1,
                    opener,
                }));
                i = j + 1;
            }
        }
    }
    if !prose.is_empty() {
        segments.push(Segment::Prose(prose));
    }
    Ok(File { segments })
}

/// Tokenises marker content: bare words and `key=value` pairs, values
/// optionally double-quoted with `\"` as the only escape. Stops at a bare `|`.
fn tokenise(line: usize, content: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' || chars[i] == '\t' {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i] != ' ' && chars[i] != '\t' && chars[i] != '=' {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        if word == "|" {
            let rest: String = chars[i..].iter().collect();
            if rest.trim_matches([' ', '\t']) != OPENER_SUFFIX.trim_start_matches("| ") {
                return Err(error(
                    line,
                    format!(
                        "unexpected text after |: only the suffix {OPENER_SUFFIX:?} may follow the attributes"
                    ),
                ));
            }
            break;
        }
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            if word.is_empty() {
                return Err(error(line, "attribute without a key"));
            }
            let value = if i < chars.len() && chars[i] == '"' {
                i += 1;
                let mut v = String::new();
                loop {
                    let Some(&c) = chars.get(i) else {
                        return Err(error(
                            line,
                            format!("unterminated quoted value for {word}="),
                        ));
                    };
                    i += 1;
                    match c {
                        '"' => break,
                        '\\' if chars.get(i) == Some(&'"') => {
                            v.push('"');
                            i += 1;
                        }
                        c => v.push(c),
                    }
                }
                if i < chars.len() && chars[i] != ' ' && chars[i] != '\t' {
                    return Err(error(
                        line,
                        format!("text after the closing quote of {word}="),
                    ));
                }
                v
            } else {
                let vstart = i;
                while i < chars.len() && chars[i] != ' ' && chars[i] != '\t' {
                    i += 1;
                }
                chars[vstart..i].iter().collect()
            };
            if value.contains("-->") {
                return Err(error(line, format!("value of {word}= contains -->")));
            }
            tokens.push(Token::Attr(word, value));
        } else {
            tokens.push(Token::Bare(word));
        }
    }
    Ok(tokens)
}

/// Parses an opener's content, what sits between `<!-- computed` and `-->`,
/// for an opener built rather than read, such as a recipe's expansion.
pub fn opener(line: usize, content: &str) -> Result<Opener, ParseError> {
    parse_opener(line, content)
}

fn parse_opener(line: usize, content: &str) -> Result<Opener, ParseError> {
    let tokens = tokenise(line, content)?;
    let mut iter = tokens.iter();
    let loader = match iter.next() {
        Some(Token::Bare(w)) => w.clone(),
        Some(Token::Attr(k, _)) => {
            return Err(error(
                line,
                format!("missing loader: the first token is {k}="),
            ));
        }
        None => return Err(error(line, "missing loader")),
    };
    let Some(grammar) = GRAMMAR.iter().find(|g| g.name == loader) else {
        return Err(error(line, format!("unknown loader {loader:?}")));
    };
    let mut flags = Vec::new();
    let mut attrs = Vec::new();
    let mut name = None;
    let mut sink = grammar.sink;
    let mut lang = String::new();
    let mut on_stale = OnStale::Fail;
    let mut max_lines = None;
    let mut seen: Vec<&str> = Vec::new();
    // `delim=` and `from=`, which only `as=table` takes.
    let mut table: Vec<(&str, &str)> = Vec::new();
    for t in iter {
        match t {
            Token::Bare(w) => {
                if !grammar.flags.contains(&w.as_str()) {
                    return Err(error(
                        line,
                        format!("unknown flag {w:?} for loader {loader}"),
                    ));
                }
                if flags.contains(w) {
                    return Err(error(line, format!("duplicate flag {w:?}")));
                }
                flags.push(w.clone());
            }
            Token::Attr(k, v) => {
                if seen.contains(&k.as_str()) {
                    return Err(error(line, format!("duplicate attribute {k}=")));
                }
                seen.push(k);
                match k.as_str() {
                    "name" => name = Some(v.clone()),
                    "as" => {
                        sink = Sink::parse(v)
                            .ok_or_else(|| error(line, format!("unknown sink {v:?}")))?;
                    }
                    "lang" => lang = v.clone(),
                    "on-stale" => {
                        on_stale = match v.as_str() {
                            "warn" => OnStale::Warn,
                            _ => {
                                return Err(error(
                                    line,
                                    format!("on-stale={v}: the only value is warn"),
                                ));
                            }
                        };
                    }
                    "max-lines" => {
                        max_lines = match v.parse::<usize>() {
                            Ok(0) => {
                                return Err(error(line, "max-lines=0: expected at least 1 line"));
                            }
                            Ok(n) => Some(n),
                            Err(_) => {
                                return Err(error(
                                    line,
                                    format!("max-lines={v}: expected a whole number"),
                                ));
                            }
                        };
                    }
                    _ if grammar.attrs.contains(&k.as_str()) => attrs.push((k.clone(), v.clone())),
                    k if SINK_ATTRS.contains(&k) => table.push((k, v)),
                    _ => {
                        return Err(error(
                            line,
                            format!("unknown attribute {k}= for loader {loader}"),
                        ));
                    }
                }
            }
        }
    }
    match (sink, table.first()) {
        (Sink::Table(_), _) => {
            let attr = |key| table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
            sink = Sink::Table(
                TableFrom::parse(attr("delim"), attr("from")).map_err(|e| error(line, e))?,
            );
        }
        // A `use` region may set them for its recipe's table; the
        // expansion is checked as a whole.
        (_, Some(_)) if loader == "use" => {}
        (_, Some((k, _))) => {
            return Err(error(
                line,
                format!(
                    "unknown attribute {k}= for loader {loader}: it applies only with as=table"
                ),
            ));
        }
        _ => {}
    }
    debug_assert!(COMMON_ATTRS.iter().all(|c| !grammar.attrs.contains(c)));
    let opener = Opener {
        loader,
        flags,
        attrs,
        name,
        sink,
        lang,
        on_stale,
        max_lines,
        written: None,
        tokens,
    };
    validate(line, &opener)?;
    Ok(opener)
}

/// The loader-specific rules the grammar owns: required attributes, numeric
/// values, and exec's exactly-one-of `inputs=` and `volatile`.
fn validate(line: usize, opener: &Opener) -> Result<(), ParseError> {
    let whole_number = |key: &str| match opener.attr(key) {
        Some(v) if v.parse::<u64>().is_err() => {
            Err(error(line, format!("{key}={v}: expected a whole number")))
        }
        _ => Ok(()),
    };
    match opener.loader.as_str() {
        "tree" => whole_number("depth"),
        "exec" => {
            if opener.attr("cmd").is_none() {
                return Err(error(line, "exec needs cmd="));
            }
            match (opener.attr("inputs").is_some(), opener.flag("volatile")) {
                (true, true) => {
                    return Err(error(line, "exec takes inputs= or volatile, not both"));
                }
                (false, false) => {
                    return Err(error(line, "exec needs inputs= or the volatile flag"));
                }
                _ => {}
            }
            if let Some(inputs) = opener.attr("inputs")
                && inputs
                    .split(',')
                    .any(|g| g.trim().trim_end_matches('/').is_empty())
            {
                return Err(error(
                    line,
                    format!("inputs={inputs}: an entry is empty; remove the stray comma"),
                ));
            }
            if let Some(inputs) = opener.attr("inputs") {
                for entry in inputs.split(',') {
                    crate::project::split_input(entry.trim())
                        .map_err(|e| error(line, format!("inputs={e}")))?;
                }
            }
            whole_number("timeout")?;
            if opener.attr("timeout").and_then(|t| t.parse::<u64>().ok()) == Some(0) {
                return Err(error(line, "timeout=0: expected at least 1 second"));
            }
            Ok(())
        }
        "file" => match opener.attr("src") {
            None => Err(error(line, "file needs src=")),
            Some(_) => crate::project::from_attrs(&opener.attrs, crate::project::FILE_SLICES)
                .map(|_| ())
                .map_err(|e| error(line, e)),
        },
        "value" => match (opener.attr("src"), opener.attr("key")) {
            (None, _) => Err(error(line, "value needs src=")),
            (_, None) => Err(error(line, "value needs key=")),
            (Some(_), Some(key)) => crate::project::Projection::parse("key", key)
                .map(|_| ())
                .map_err(|e| error(line, e)),
        },
        "index" => {
            let Some(src) = opener.attr("src") else {
                return Err(error(line, "index needs src="));
            };
            for glob in src.split(',') {
                if glob.trim().trim_end_matches('/').is_empty() {
                    return Err(error(
                        line,
                        format!("src={src}: an entry is empty; remove the stray comma"),
                    ));
                }
                if let Ok((_, Some(_))) | Err(_) = crate::project::split_input(glob) {
                    return Err(error(
                        line,
                        format!("src={src}: index takes globs, not a projection"),
                    ));
                }
            }
            match opener.attr("title") {
                Some(t) if crate::index::Title::parse(t).is_none() => {
                    Err(error(line, format!("title={t}: expected h1 or filename")))
                }
                _ => Ok(()),
            }
        }
        "toc" => crate::toc::levels(opener.attr("min"), opener.attr("max"))
            .map(|_| ())
            .map_err(|e| error(line, e)),
        "use" => match opener.attr("recipe") {
            None => Err(error(line, "use needs recipe=")),
            Some(_) => Ok(()),
        },
        _ => Ok(()),
    }
}

fn parse_closer(line: usize, content: &str) -> Result<Option<Sums>, ParseError> {
    let tokens = tokenise(line, content)?;
    let mut input = None;
    let mut output = None;
    for t in tokens {
        match t {
            Token::Bare(w) => return Err(error(line, format!("unknown flag {w:?} in closer"))),
            Token::Attr(k, v) => {
                let slot = match k.as_str() {
                    "in" => &mut input,
                    "out" => &mut output,
                    _ => return Err(error(line, format!("unknown attribute {k}= in closer"))),
                };
                if slot.is_some() {
                    return Err(error(line, format!("duplicate attribute {k}= in closer")));
                }
                let ok = v.len() == 64
                    && v.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if !ok {
                    return Err(error(
                        line,
                        format!("malformed sum {k}={v}: expected 64 lowercase hex characters"),
                    ));
                }
                *slot = Some(v);
            }
        }
    }
    match (input, output) {
        (Some(input), Some(output)) => Ok(Some(Sums { input, output })),
        (None, None) => Ok(None),
        _ => Err(error(
            line,
            "one sum in closer: in= and out= come together or not at all",
        )),
    }
}

/// The inverse of `parse`: prose and raw region lines concatenated.
pub fn serialise(file: &File) -> String {
    let mut out = String::new();
    for s in &file.segments {
        match s {
            Segment::Prose(p) => out.push_str(p),
            Segment::Region(r) => {
                out.push_str(&r.raw_opener);
                out.push_str(&r.body);
                out.push_str(&r.raw_closer);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(file: &File, i: usize) -> &Region {
        match &file.segments[i] {
            Segment::Region(r) => r,
            Segment::Prose(p) => panic!("segment {i} is prose: {p:?}"),
        }
    }

    #[test]
    fn a_file_without_markers_is_one_prose_segment() {
        let text = "# Title\n\nsome prose\n";
        let file = parse(text).unwrap();
        assert_eq!(file.segments.len(), 1);
        assert!(matches!(&file.segments[0], Segment::Prose(p) if p == text));
        assert_eq!(serialise(&file), text);
    }

    #[test]
    fn an_unrendered_region_parses_with_no_sums() {
        let text =
            "before\n<!-- computed tree src=. depth=2 name=layout -->\n<!-- /computed -->\nafter\n";
        let file = parse(text).unwrap();
        assert_eq!(file.segments.len(), 3);
        let r = region(&file, 1);
        assert_eq!(r.line, 2);
        assert_eq!(r.body, "");
        assert!(r.sums.is_none());
        assert_eq!(r.opener.loader, "tree");
        assert_eq!(r.opener.name.as_deref(), Some("layout"));
        assert_eq!(r.opener.sink, Sink::Fence);
        assert_eq!(
            r.opener.attrs,
            vec![
                ("src".to_string(), ".".to_string()),
                ("depth".to_string(), "2".to_string())
            ]
        );
        assert_eq!(serialise(&file), text);
    }

    #[test]
    fn a_rendered_region_keeps_its_body_bytes_and_sums() {
        let text = "  <!-- computed tree | do not edit; run computed -->\n```text\n.\n```\n  <!-- /computed in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715 -->";
        let file = parse(text).unwrap();
        let r = region(&file, 0);
        assert_eq!(r.indent, "  ");
        assert_eq!(r.body, "```text\n.\n```\n");
        let sums = r.sums.as_ref().unwrap();
        assert_eq!(
            sums.input,
            "9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60"
        );
        assert_eq!(
            sums.output,
            "41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715"
        );
        assert_eq!(
            r.raw_opener,
            "  <!-- computed tree | do not edit; run computed -->\n"
        );
        assert_eq!(
            r.raw_closer,
            "  <!-- /computed in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715 -->"
        );
        assert_eq!(serialise(&file), text);
    }

    #[test]
    fn canonical_opener_uses_single_spaces_and_drops_suffix_and_indent() {
        let text = "\t<!--   computed\ttree   src=.  all name=x   | do not edit; run computed   -->  \n<!-- /computed -->\n";
        let file = parse(text).unwrap();
        let r = region(&file, 0);
        assert_eq!(
            r.opener.canonical(),
            "<!-- computed tree src=. all name=x -->"
        );
        assert_eq!(r.opener.flags, vec!["all".to_string()]);
    }

    #[test]
    fn quoted_values_carry_whitespace_and_escaped_quotes() {
        let text = r#"<!-- computed exec cmd="grep -h '^# ' docs/*.md" inputs=docs/*.md name=adrs -->
<!-- /computed -->
"#;
        let file = parse(text).unwrap();
        let r = region(&file, 0);
        assert_eq!(
            r.opener.attrs[0],
            ("cmd".to_string(), "grep -h '^# ' docs/*.md".to_string())
        );
        assert_eq!(r.opener.sink, Sink::Raw);
        assert_eq!(
            r.opener.canonical(),
            r#"<!-- computed exec cmd="grep -h '^# ' docs/*.md" inputs=docs/*.md name=adrs -->"#
        );

        let text = r#"<!-- computed exec cmd="say \"hi\"" volatile -->
<!-- /computed -->
"#;
        let r = parse(text).unwrap();
        let r = region(&r, 0);
        assert_eq!(r.opener.attrs[0].1, "say \"hi\"");
        assert_eq!(
            r.opener.canonical(),
            r#"<!-- computed exec cmd="say \"hi\"" volatile -->"#
        );
    }

    #[test]
    fn as_and_lang_select_the_sink() {
        let text =
            "<!-- computed exec cmd=date volatile as=fence lang=text -->\n<!-- /computed -->\n";
        let r = parse(text).unwrap();
        let r = region(&r, 0);
        assert_eq!(r.opener.sink, Sink::Fence);
        assert_eq!(r.opener.lang, "text");
        assert!(r.opener.attrs.iter().all(|(k, _)| k != "as" && k != "lang"));
    }

    #[test]
    fn markers_inside_fenced_code_blocks_are_prose() {
        let text = "````markdown\n<!-- computed tree -->\n<!-- /computed -->\n````\n~~~\n<!-- computed tree -->\n~~~\n";
        let file = parse(text).unwrap();
        assert_eq!(file.segments.len(), 1);
        assert!(matches!(&file.segments[0], Segment::Prose(_)));
    }

    #[test]
    fn an_unclosed_fence_is_prose_and_hides_nothing() {
        let text = "```\nstray fence\n<!-- computed tree name=x -->\n<!-- /computed -->\n";
        let file = parse(text).unwrap();
        assert_eq!(file.segments.len(), 2);
        assert_eq!(region(&file, 1).line, 3);
        let text = "~~~\n<!-- computed tree name=x -->\n<!-- /computed -->\n```\n";
        assert_eq!(
            parse(text).unwrap().segments.len(),
            3,
            "a backtick fence does not close a tilde one"
        );
    }

    #[test]
    fn a_fence_inside_a_body_hides_a_closer_look_alike() {
        let text = "<!-- computed exec cmd=x volatile -->\n\n````\n<!-- /computed -->\n````\n\n<!-- /computed in=0000000000000000000000000000000000000000000000000000000000000000 out=0000000000000000000000000000000000000000000000000000000000000000 -->\n";
        let file = parse(text).unwrap();
        let r = region(&file, 0);
        assert_eq!(r.body, "\n````\n<!-- /computed -->\n````\n\n");
        assert_eq!(serialise(&file), text);
    }

    #[test]
    fn two_regions_and_prose_between_round_trip() {
        let text = "a\n<!-- computed tree name=one -->\nbody1\n<!-- /computed -->\nmid\n<!-- computed exec cmd=x volatile name=two -->\n<!-- /computed -->\n";
        let file = parse(text).unwrap();
        assert_eq!(file.segments.len(), 4);
        assert_eq!(region(&file, 1).line, 2);
        assert_eq!(region(&file, 3).line, 6);
        assert_eq!(serialise(&file), text);
    }

    fn err(text: &str) -> ParseError {
        parse(text).expect_err("expected a parse error")
    }

    #[test]
    fn every_grammar_error_is_reported_with_its_line() {
        let cases: &[(&str, usize, &str)] = &[
            (
                "<!-- computed csv src=a -->\n<!-- /computed -->\n",
                1,
                "unknown loader",
            ),
            (
                "<!-- computed tree bogus=1 -->\n<!-- /computed -->\n",
                1,
                "unknown attribute",
            ),
            (
                "<!-- computed tree bogus -->\n<!-- /computed -->\n",
                1,
                "unknown flag",
            ),
            (
                "<!-- computed tree src=. src=. -->\n<!-- /computed -->\n",
                1,
                "duplicate attribute",
            ),
            (
                "<!-- computed -->\n<!-- /computed -->\n",
                1,
                "missing loader",
            ),
            (
                "<!-- computed tree name=a -->\n<!-- /computed -->\n<!-- computed tree name=a -->\n<!-- /computed -->\n",
                3,
                "duplicate name",
            ),
            (
                "x\n<!-- computed tree -->\nbody\n",
                2,
                "opener without closer",
            ),
            ("x\n<!-- /computed -->\n", 2, "closer without opener"),
            (
                "<!-- computed tree -->\n<!-- computed tree -->\n<!-- /computed -->\n",
                2,
                "opener inside a body",
            ),
            (
                "<!-- computed tree -->\n<!-- /computed in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 -->\n",
                2,
                "one sum",
            ),
            (
                "<!-- computed tree -->\n<!-- /computed in=9f3a out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715 -->\n",
                2,
                "malformed sum",
            ),
            (
                "<!-- computed tree -->\n<!-- /computed in=9f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715 -->\n",
                2,
                "malformed sum",
            ),
            (
                "<!-- computed tree -->\n<!-- /computed in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f6g out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715 -->\n",
                2,
                "malformed sum",
            ),
            (
                "<!-- computed tree -->\n<!-- /computed extra=1 -->\n",
                2,
                "unknown attribute",
            ),
            (
                "<!-- computed exec cmd=\"a --> b\" volatile -->\n<!-- /computed -->\n",
                1,
                "-->",
            ),
            (
                "<!-- computed exec cmd=\"unterminated volatile -->\n<!-- /computed -->\n",
                1,
                "unterminated",
            ),
            (
                "<!-- computed tree as=bogus -->\n<!-- /computed -->\n",
                1,
                "unknown sink",
            ),
            (
                "<!-- computed tree src=.\n<!-- /computed -->\n",
                1,
                "unterminated marker",
            ),
            (
                "<!-- computed tree | whatever -->\n<!-- /computed -->\n",
                1,
                "after |",
            ),
            (
                "<!-- computed tree depth=two -->\n<!-- /computed -->\n",
                1,
                "depth=two",
            ),
            (
                "<!-- computed exec inputs=a -->\n<!-- /computed -->\n",
                1,
                "cmd=",
            ),
            (
                "<!-- computed exec cmd=x -->\n<!-- /computed -->\n",
                1,
                "volatile",
            ),
            (
                "<!-- computed exec cmd=x inputs=a volatile -->\n<!-- /computed -->\n",
                1,
                "not both",
            ),
            (
                "<!-- computed exec cmd=x volatile timeout=1s -->\n<!-- /computed -->\n",
                1,
                "timeout=1s",
            ),
            (
                "<!-- computed exec cmd=x volatile timeout=0 -->\n<!-- /computed -->\n",
                1,
                "at least 1 second",
            ),
            (
                "<!-- computed exec cmd=x inputs=docs/*.md, -->\n<!-- /computed -->\n",
                1,
                "entry is empty",
            ),
            (
                "<!-- computed exec cmd=x inputs=a,,b -->\n<!-- /computed -->\n",
                1,
                "entry is empty",
            ),
            (
                "<!-- computed file -->\n<!-- /computed -->\n",
                1,
                "file needs src=",
            ),
            (
                "<!-- computed file src=a volatile -->\n<!-- /computed -->\n",
                1,
                "unknown flag",
            ),
        ];
        for (text, line, needle) in cases {
            let e = err(text);
            assert_eq!(e.line, *line, "line for {text:?}: {}", e.message);
            assert!(
                e.message.contains(needle),
                "{text:?}: expected {needle:?} in {:?}",
                e.message
            );
        }
    }

    #[test]
    fn a_closer_may_touch_its_comment_end() {
        let text = "<!-- computed tree-->\nbody\n<!-- /computed-->\n";
        let file = parse(text).unwrap();
        assert_eq!(region(&file, 0).body, "body\n");
        assert_eq!(serialise(&file), text);
    }

    #[test]
    fn unclosed_fences_are_found_and_closed_ones_are_not() {
        assert_eq!(unclosed_fences("a\n```\nb\n~~~\nc\n~~~\n"), [2]);
        assert!(unclosed_fences("```\nx\n```\n").is_empty());
    }

    #[test]
    fn a_fenced_marker_is_not_an_unfenced_one() {
        assert_eq!(unfenced_marker_line("```\n<!-- /computed -->\n```\n"), None);
        assert_eq!(
            unfenced_marker_line("x\n  <!-- /computed -->\n"),
            Some("  <!-- /computed -->")
        );
    }

    #[test]
    fn strip_sums_blanks_closers_and_nothing_else() {
        let sums = "in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715";
        let text = format!("a /computed b\r\n  <!-- /computed {sums} -->\r\n<!-- /computed -->\n");
        let mut bytes = text.into_bytes();
        bytes.push(0xff);
        assert_eq!(
            &*strip_sums(&bytes),
            b"a /computed b\r\n  <!-- /computed -->\r\n<!-- /computed -->\n\xff"
        );
        assert!(matches!(
            strip_sums(b"no markers"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn a_lookalike_comment_is_prose() {
        let text = "<!-- computedx tree -->\n<!--computed tree-->\n<!-- Computed tree -->\n";
        let file = parse(text).unwrap();
        assert_eq!(file.segments.len(), 1);
    }
}
