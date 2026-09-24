//! The marker grammar: parse a file into prose and regions, and serialise it back.
//!
//! A marker is a whole line: optional indentation, a comment whose first
//! token is `computed` (opener) or `/computed` (closer), optional trailing
//! whitespace. The comment is the file's own, chosen by its extension or
//! name ([`Syntax`]): `<!-- … -->` in Markdown and HTML, `//`, `#` or `--`
//! line comments in code, `/* … */` in CSS. In a line comment, an opener
//! must name a known loader, so a comment that merely starts with the word
//! is prose. Markers inside CommonMark fenced code blocks of a Markdown file
//! are prose. The raw lines are kept so a fresh region can be reproduced
//! byte for byte.
//!
//! In Markdown and HTML a region may also sit inside a line: an opener
//! comment that does not stand alone on its line, with its closer later on
//! the same line, holds the text between them. In Markdown, such markers
//! inside a code span are prose.

use std::fmt;
use std::path::Path;

use crate::table::TableFrom;

/// A parsed template: prose and regions in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub segments: Vec<Segment>,
    /// The comment syntax the file was read in.
    pub syntax: Syntax,
}

/// The comment a marker is written in, as the file spells it: the token
/// that opens it and, for a comment that ends on its line, the one that
/// closes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Comment {
    pub open: &'static str,
    /// Empty for a line comment, which runs to the end of its line.
    pub close: &'static str,
}

impl Comment {
    pub const HTML: Comment = Comment {
        open: "<!--",
        close: "-->",
    };
    const CSS: Comment = Comment {
        open: "/*",
        close: "*/",
    };
    const SLASH: Comment = Comment::line("//");
    /// Rust's outer and inner doc comments, which `//` alone would miss.
    const DOC: Comment = Comment::line("///");
    const INNER_DOC: Comment = Comment::line("//!");
    const HASH: Comment = Comment::line("#");
    const DASH: Comment = Comment::line("--");

    const fn line(open: &'static str) -> Comment {
        Comment { open, close: "" }
    }

    /// Whether the comment runs to the end of its line, so a sink can put
    /// its leader before every body line.
    pub fn is_line(self) -> bool {
        self.close.is_empty()
    }

    /// `content` as a marker in this comment, one space either side.
    pub fn wrap(self, content: &str) -> String {
        if self.is_line() {
            format!("{} {content}", self.open)
        } else {
            format!("{} {content} {}", self.open, self.close)
        }
    }
}

/// The comment syntax of a file, chosen by its extension or name. Markdown
/// is the only one with fenced code blocks, and the only one whose loaders
/// default to a fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Syntax {
    /// `<!-- … -->`; markers inside fenced code blocks are prose.
    Markdown,
    /// `<!-- … -->` in HTML, XML and SVG, with no fence rule.
    Html,
    /// `//` line comments, and Rust's `///` and `//!`.
    Slash,
    /// `#` line comments.
    Hash,
    /// `--` line comments.
    Dash,
    /// `/* … */` on one line.
    Css,
}

/// Every extension with a syntax. An extension compares without case.
const EXTENSIONS: &[(Syntax, &[&str])] = &[
    (Syntax::Markdown, &["md", "markdown"]),
    (Syntax::Html, &["html", "htm", "xml", "svg"]),
    (
        Syntax::Slash,
        &[
            "rs", "js", "mjs", "cjs", "jsx", "ts", "tsx", "go", "c", "h", "cc", "cpp", "hpp",
            "java", "kt", "swift", "scala", "cs", "dart", "zig", "proto",
        ],
    ),
    (
        Syntax::Hash,
        &[
            "py", "sh", "bash", "zsh", "rb", "toml", "yaml", "yml", "pl", "r", "tf", "nix", "ini",
            "cfg",
        ],
    ),
    (Syntax::Dash, &["sql", "lua", "hs"]),
    (Syntax::Css, &["css"]),
];

/// File names with a syntax and no extension to tell it by.
const NAMES: &[(&str, Syntax)] = &[
    ("Makefile", Syntax::Hash),
    ("GNUmakefile", Syntax::Hash),
    ("makefile", Syntax::Hash),
    ("Dockerfile", Syntax::Hash),
    ("Containerfile", Syntax::Hash),
    (".gitignore", Syntax::Hash),
    (".gitattributes", Syntax::Hash),
    (".dockerignore", Syntax::Hash),
];

impl Syntax {
    /// The syntax an extension, without its dot, selects.
    pub fn of_extension(ext: &str) -> Option<Syntax> {
        let ext = ext.to_ascii_lowercase();
        EXTENSIONS
            .iter()
            .find(|(_, exts)| exts.contains(&ext.as_str()))
            .map(|(s, _)| *s)
    }

    /// The syntax a file name without an extension to tell it by selects.
    pub fn of_name(name: &str) -> Option<Syntax> {
        NAMES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
    }

    /// The syntax a path selects by its name, else its extension.
    pub fn of(path: &Path) -> Option<Syntax> {
        let name = path.file_name()?.to_str()?;
        Syntax::of_name(name).or_else(|| Syntax::of_extension(path.extension()?.to_str()?))
    }

    /// The syntax a file is read in: an unknown extension reads as
    /// Markdown, as every file named on the command line always did.
    pub fn for_path(path: &Path) -> Syntax {
        Syntax::of(path).unwrap_or(Syntax::Markdown)
    }

    /// Every extension and file name with a syntax, for a message.
    pub fn supported() -> String {
        let exts: Vec<&str> = EXTENSIONS
            .iter()
            .flat_map(|(_, e)| e.iter().copied())
            .collect();
        let names: Vec<&str> = NAMES.iter().map(|(n, _)| *n).collect();
        format!(
            "the extensions {} and the names {}",
            exts.join(" "),
            names.join(" ")
        )
    }

    /// The comments a marker may be written in, longest opening token first.
    fn comments(self) -> &'static [Comment] {
        match self {
            Syntax::Markdown | Syntax::Html => &[Comment::HTML],
            Syntax::Slash => &[Comment::DOC, Comment::INNER_DOC, Comment::SLASH],
            Syntax::Hash => &[Comment::HASH],
            Syntax::Dash => &[Comment::DASH],
            Syntax::Css => &[Comment::CSS],
        }
    }

    pub fn is_markdown(self) -> bool {
        self == Syntax::Markdown
    }

    /// Whether a region may sit inside a line: only where the comment ends
    /// on its line and a reader does not see it, in Markdown and HTML.
    pub fn inline(self) -> bool {
        matches!(self, Syntax::Markdown | Syntax::Html)
    }

    /// Whether `text` could hold a marker at all: a quick test before a parse.
    pub fn may_hold(self, text: &str) -> bool {
        match self {
            Syntax::Markdown | Syntax::Html => text.contains("<!--"),
            _ => text.contains("computed"),
        }
    }
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
    /// The syntax of the file the region sits in.
    pub syntax: Syntax,
    /// The comment the opener is written in, which the tool writes both
    /// markers in.
    pub comment: Comment,
    /// For a region inside a line, the 1-based column, in characters, its
    /// opener starts at; `None` for a region on lines of its own. An inline
    /// region's markers are the comments alone and its body the text
    /// between them, with no line terminator anywhere.
    pub column: Option<usize>,
}

impl Region {
    /// Where the opener sits: its line, and its column when inline. What
    /// tells two regions on one line apart.
    pub fn at(&self) -> (usize, Option<usize>) {
        (self.line, self.column)
    }

    /// `line`, or `line:column` for an inline region, as reports print it.
    pub fn place(&self) -> String {
        place(self.line, self.column)
    }

    /// The 1-based line of the closer.
    pub fn last_line(&self) -> usize {
        match self.column {
            Some(_) => self.line,
            None => self.line + 1 + self.body.matches('\n').count(),
        }
    }
}

/// `line`, or `line:column` when there is a column, as reports print a
/// region's place.
pub fn place(line: usize, column: Option<usize>) -> String {
    match column {
        Some(c) => format!("{line}:{c}"),
        None => line.to_string(),
    }
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
    /// `as=comment`: every line behind the opener's line-comment leader.
    Comment,
}

impl Sink {
    fn parse(s: &str) -> Option<Sink> {
        match s {
            "raw" => Some(Sink::Raw),
            "fence" => Some(Sink::Fence),
            "comment" => Some(Sink::Comment),
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
    /// The recipe it was expanded from.
    recipe: Option<String>,
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

    /// The canonical form of the `use` opener this one was expanded from,
    /// which the file shows; `None` for an opener as written.
    pub fn written(&self) -> Option<&str> {
        self.written.as_deref()
    }

    /// The recipe this opener was expanded from; `None` for one as written.
    pub fn recipe(&self) -> Option<&str> {
        self.recipe.as_deref()
    }

    /// This opener standing in for the `use` opener `written`: the file
    /// keeps showing `written`, and everything else reads this one.
    pub fn expanded_from(mut self, written: &Opener) -> Opener {
        self.written = Some(written.canonical());
        self.recipe = written.attr("recipe").map(str::to_string);
        self
    }

    /// The opener as it reads in a file of `syntax`, inside a line when
    /// `inline`: a loader that fences its text by default writes it raw
    /// outside Markdown, where a fence means nothing, and `as=comment`
    /// needs a comment with a line leader. An inline region writes its
    /// text as it stands: any other sink, and `max-lines=`, are errors.
    pub fn placed(mut self, syntax: Syntax, inline: bool) -> Result<Opener, String> {
        let written_as = self.attr_written("as").map(str::to_string);
        if inline {
            if let Some(sink) = written_as.as_ref().filter(|_| self.sink != Sink::Raw) {
                return Err(format!(
                    "as={sink}: a region inside a line writes its text as it stands, so it takes only as=raw"
                ));
            }
            if self.max_lines.is_some() {
                return Err(
                    "max-lines= cuts lines, and a region inside a line holds one".to_string(),
                );
            }
            self.sink = Sink::Raw;
        }
        if written_as.is_none() && self.sink == Sink::Fence && !syntax.is_markdown() {
            self.sink = Sink::Raw;
        }
        let comment = syntax.comments()[0];
        if self.sink == Sink::Comment && !comment.is_line() {
            return Err(format!(
                "as=comment needs a line comment such as // or #; a {} {} comment has no leader to put before each line",
                comment.open, comment.close
            ));
        }
        Ok(self)
    }

    /// A token's value as written, common attributes included.
    fn attr_written(&self, key: &str) -> Option<&str> {
        self.tokens.iter().find_map(|t| match t {
            Token::Attr(k, v) if k == key => Some(v.as_str()),
            _ => None,
        })
    }

    /// The opener with the loader attribute `key=` set to `value`: in its
    /// place when written, else last.
    pub fn with_attr(&self, key: &str, value: &str) -> Opener {
        let mut out = self.clone();
        let token = out
            .tokens
            .iter_mut()
            .find(|t| matches!(t, Token::Attr(k, _) if k == key));
        match token {
            Some(Token::Attr(_, v)) => *v = value.to_string(),
            _ => out.tokens.push(Token::Attr(key.into(), value.into())),
        }
        match out.attrs.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value.to_string(),
            None => out.attrs.push((key.into(), value.into())),
        }
        out
    }
}

/// The suffix the tool writes after the attributes of a rendered opener.
pub const OPENER_SUFFIX: &str = "| do not edit; run computed";

/// The attributes of a canonical opener: what sits between `<!--` and `-->`.
fn content(canonical: &str) -> &str {
    canonical
        .strip_prefix("<!-- ")
        .and_then(|c| c.strip_suffix(" -->"))
        .expect("canonical opener is an HTML comment")
}

/// The canonical form in `comment`, the `use` opener it was expanded from
/// if it was, without the suffix or indent.
pub fn opener_line(opener: &Opener, comment: Comment) -> String {
    let c = opener.written.clone().unwrap_or_else(|| opener.canonical());
    comment.wrap(content(&c))
}

/// The rendered opener line in `comment`: [`opener_line`] with the suffix.
pub fn rendered_opener(opener: &Opener, comment: Comment) -> String {
    let c = opener.written.clone().unwrap_or_else(|| opener.canonical());
    comment.wrap(&format!("{} {OPENER_SUFFIX}", content(&c)))
}

/// The closer line in `comment` for the given sums, without indent.
pub fn rendered_closer(sums: Option<&Sums>, comment: Comment) -> String {
    match sums {
        Some(s) => comment.wrap(&format!("/computed in={} out={}", s.input, s.output)),
        None => comment.wrap("/computed"),
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
        flags: &["volatile", "sandbox"],
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
    LoaderGrammar {
        name: "symbol",
        attrs: &["src", "item", "part"],
        flags: &[],
        sink: Sink::Fence,
    },
    LoaderGrammar {
        name: "git",
        attrs: &["src", "n"],
        flags: &["log", "tags", "contributors"],
        sink: Sink::Raw,
    },
    LoaderGrammar {
        name: "remote",
        attrs: &["url", "sha256", "timeout"],
        flags: &[],
        sink: Sink::Raw,
    },
    LoaderGrammar {
        name: "transcript",
        attrs: &["steps", "inputs", "timeout", "workdir"],
        flags: &["volatile", "sandbox"],
        sink: Sink::Fence,
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
    /// The content between the comment's tokens after the first word,
    /// trimmed, plus the indent and the comment.
    Opener {
        indent: &'a str,
        content: &'a str,
        comment: Comment,
    },
    Closer {
        indent: &'a str,
        content: &'a str,
        comment: Comment,
    },
}

/// Whether `word` names a loader, so a line comment that starts with
/// `computed` and then prose is prose.
fn is_loader(word: &str) -> bool {
    GRAMMAR.iter().any(|g| g.name == word)
}

fn classify<'a>(line: &Line<'a>, syntax: Syntax) -> Result<Kind<'a>, ParseError> {
    let text = line.text;
    let trimmed_start = text.trim_start_matches([' ', '\t']);
    let indent = &text[..text.len() - trimmed_start.len()];
    let found = syntax.comments().iter().find_map(|&c| {
        let after = trimmed_start.strip_prefix(c.open)?;
        after.starts_with([' ', '\t']).then_some((c, after))
    });
    let Some((comment, after)) = found else {
        return Ok(Kind::Prose);
    };
    let inner = after.trim_start_matches([' ', '\t']);
    let word_end = inner.find([' ', '\t']).unwrap_or(inner.len());
    // `<!-- /computed-->` closes as surely as `<!-- /computed -->`.
    let word = &inner[..word_end];
    let word = if comment.is_line() {
        word
    } else {
        word.strip_suffix(comment.close).unwrap_or(word)
    };
    if word != "computed" && word != "/computed" {
        return Ok(Kind::Prose);
    }
    if comment.is_line() && word == "computed" {
        let loader = inner[word.len()..]
            .trim_start_matches([' ', '\t'])
            .split([' ', '\t'])
            .next()
            .unwrap_or("");
        if !is_loader(loader) {
            return Ok(Kind::Prose);
        }
    }
    let body = trimmed_start.trim_end_matches([' ', '\t']);
    let content = if comment.is_line() {
        body
    } else {
        match comment_end(body.as_bytes(), comment) {
            Some(end) if end + comment.close.len() == body.len() => &body[..end],
            // Text after the comment: a region inside the line, which the
            // line's own scan finds.
            Some(_) if syntax.inline() => return Ok(Kind::Prose),
            Some(_) => {
                return Err(error(
                    line.number,
                    format!(
                        "text after the marker's {}: a marker is a whole line",
                        comment.close
                    ),
                ));
            }
            None => {
                return Err(error(
                    line.number,
                    format!(
                        "unterminated marker: the line does not end with {}",
                        comment.close
                    ),
                ));
            }
        }
    };
    let content = content[comment.open.len()..].trim_matches([' ', '\t']);
    let content = content[word.len()..].trim_start_matches([' ', '\t']);
    // `-->` in a value is the tokeniser's to refuse; another comment's
    // end would end the marker early wherever the file is read.
    if !comment.is_line() && comment != Comment::HTML && content.contains(comment.close) {
        return Err(error(
            line.number,
            format!(
                "the marker holds {} before its end, which would end the comment there",
                comment.close
            ),
        ));
    }
    if word == "computed" {
        Ok(Kind::Opener {
            indent,
            content,
            comment,
        })
    } else {
        Ok(Kind::Closer {
            indent,
            content,
            comment,
        })
    }
}

/// Where the comment at the start of `s` ends: the byte index of its
/// closing token. A quoted value is skipped as the tokeniser reads it, so a
/// `-->` inside one is the tokeniser's error to name.
fn comment_end(s: &[u8], comment: Comment) -> Option<usize> {
    let close = comment.close.as_bytes();
    let mut i = comment.open.len();
    while i < s.len() {
        if s[i] == b'=' && s.get(i + 1) == Some(&b'"') {
            i += 2;
            while i < s.len() && s[i] != b'"' {
                i += if s[i] == b'\\' && s.get(i + 1) == Some(&b'"') {
                    2
                } else {
                    1
                };
            }
            i += 1;
            continue;
        }
        if s[i..].starts_with(close) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// A marker comment inside a line: its byte range in the line, and its
/// content after the first word.
struct Inline<'a> {
    start: usize,
    end: usize,
    opener: bool,
    content: &'a str,
}

/// The marker comments inside `line`, outside the code spans `code` covers,
/// in order. Another comment is skipped whole. A marker comment that does
/// not end on the line is an error: a region inside a line ends on it.
fn inline_markers<'a>(
    line: &Line<'a>,
    code: &[(usize, usize)],
) -> Result<Vec<Inline<'a>>, ParseError> {
    let text = line.text;
    let bytes = text.as_bytes();
    let open = Comment::HTML.open.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i..].starts_with(open) || code.iter().any(|&(a, b)| (a..b).contains(&i)) {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        let after = &rest[open.len()..];
        let inner = after.trim_start_matches([' ', '\t']);
        let word = &inner[..inner.find([' ', '\t']).unwrap_or(inner.len())];
        let word = word.split("-->").next().unwrap_or(word);
        let marker = after.starts_with([' ', '\t']) && (word == "computed" || word == "/computed");
        let Some(end) = comment_end(rest.as_bytes(), Comment::HTML) else {
            if marker {
                return Err(error(
                    line.number,
                    "unterminated marker: the comment does not end on its line",
                ));
            }
            break;
        };
        if marker {
            let content = rest[open.len()..end].trim_matches([' ', '\t']);
            found.push(Inline {
                start: i,
                end: i + end + Comment::HTML.close.len(),
                opener: word == "computed",
                content: content[word.len()..].trim_start_matches([' ', '\t']),
            });
        }
        i += end + Comment::HTML.close.len();
    }
    Ok(found)
}

/// Whether a line begins a Markdown block of its own, so a code span
/// cannot run into it from the line above: a heading, a quote, a list
/// item, a table row.
fn starts_block(text: &str) -> bool {
    let t = text.trim_start_matches(' ');
    if text.len() - t.len() > 3 {
        return false;
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    t.starts_with(['#', '>', '|'])
        || ["- ", "* ", "+ "].iter().any(|m| t.starts_with(m))
        || (digits > 0 && [". ", ") "].iter().any(|m| t[digits..].starts_with(m)))
}

/// Per line of a Markdown file, the byte ranges code spans cover. A span
/// is a backtick run closed by the next run of the same length within the
/// paragraph, which is the lines between blank lines, fences, whole-line
/// markers and the starts of other blocks. An HTML comment is skipped whole,
/// so backticks in a marker's attributes open nothing.
fn code_spans(lines: &[Line<'_>], fenced: &[bool], syntax: Syntax) -> Vec<Vec<(usize, usize)>> {
    let mut spans = vec![Vec::new(); lines.len()];
    if !syntax.is_markdown() {
        return spans;
    }
    let apart = |i: usize| {
        fenced[i]
            || lines[i].text.trim().is_empty()
            || !matches!(classify(&lines[i], syntax), Ok(Kind::Prose))
    };
    let mut i = 0;
    while i < lines.len() {
        if apart(i) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < lines.len()
            && !apart(j)
            && !starts_block(lines[j].text)
            && !lines[j - 1].text.trim_start().starts_with('#')
        {
            j += 1;
        }
        // Only a paragraph that holds a comment needs its spans.
        if !lines[i..j].iter().any(|l| l.text.contains("<!--")) {
            i = j;
            continue;
        }
        let mut starts = Vec::new();
        let mut para = String::new();
        for l in &lines[i..j] {
            starts.push(para.len());
            para.push_str(l.text);
            para.push('\n');
        }
        let b = para.as_bytes();
        let run = |at: usize| b[at..].iter().take_while(|&&c| c == b'`').count();
        let mut k = 0;
        while k < b.len() {
            if b[k] == b'\\' {
                k += 2;
            } else if b[k] == b'`' {
                let n = run(k);
                let mut m = k + n;
                let mut close = None;
                while m < b.len() {
                    if b[m] == b'`' {
                        let r = run(m);
                        if r == n {
                            close = Some(m + r);
                            break;
                        }
                        m += r;
                    } else {
                        m += 1;
                    }
                }
                match close {
                    Some(end) => {
                        for (line, &s) in starts.iter().enumerate() {
                            let e = s + lines[i + line].text.len();
                            if k < e && end > s {
                                spans[i + line].push((k.max(s) - s, end.min(e) - s));
                            }
                        }
                        k = end;
                    }
                    None => k += n,
                }
            } else if b[k..].starts_with(b"<!--") {
                k = match comment_end(&b[k..], Comment::HTML) {
                    Some(e) => k + e + Comment::HTML.close.len(),
                    None => b.len(),
                };
            } else {
                k += 1;
            }
        }
        i = j;
    }
    spans
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

/// Which lines hold no marker whatever they say: in Markdown, the lines of
/// a fenced code block; in HTML, those of a `<script>` or `<style>`
/// element, whose text is not HTML, so `<!--` there opens no comment. No
/// other syntax has such lines.
fn fenced_in(lines: &[Line<'_>], syntax: Syntax) -> Vec<bool> {
    match syntax {
        Syntax::Markdown => fenced_lines(lines),
        Syntax::Html => raw_text_lines(lines),
        _ => vec![false; lines.len()],
    }
}

/// The lines from one holding a `<script>` or `<style>` start tag to the
/// one holding its end tag, both included. An element never closed is
/// read to the end of the file, as a browser reads it.
fn raw_text_lines(lines: &[Line<'_>]) -> Vec<bool> {
    let opens = |text: &str| -> Option<&'static str> {
        let lower = text.to_ascii_lowercase();
        ["script", "style"].into_iter().find(|tag| {
            lower.match_indices(&format!("<{tag}")).any(|(i, m)| {
                matches!(
                    lower.as_bytes().get(i + m.len()),
                    None | Some(b'>' | b' ' | b'\t' | b'/')
                )
            })
        })
    };
    let mut raw = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let Some(tag) = opens(lines[i].text) else {
            i += 1;
            continue;
        };
        let end = format!("</{tag}");
        let close = (i..lines.len())
            .find(|&j| lines[j].text.to_ascii_lowercase().contains(&end))
            .unwrap_or(lines.len() - 1);
        raw[i..=close].iter_mut().for_each(|r| *r = true);
        i = close + 1;
    }
    raw
}

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

/// Whether a line, on its own, would parse as an opener or a closer in a
/// Markdown file.
pub fn is_marker(text: &str) -> bool {
    is_marker_in(text, Syntax::Markdown)
}

/// Whether a line, on its own, would parse as an opener or a closer.
pub fn is_marker_in(text: &str, syntax: Syntax) -> bool {
    let line = Line {
        number: 1,
        raw: text,
        text,
    };
    !matches!(classify(&line, syntax), Ok(Kind::Prose))
}

/// The first line of `text` outside a fenced code block that would parse
/// as a marker. Such a line in a `raw` body would open or close a region.
pub fn unfenced_marker_line(text: &str, syntax: Syntax) -> Option<&str> {
    let lines = lines(text);
    let fenced = fenced_in(&lines, syntax);
    lines
        .iter()
        .zip(&fenced)
        .find(|(l, f)| !**f && is_marker_in(l.text, syntax))
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

/// Whether any line of `text` would parse as a marker in `syntax`, fenced
/// or not.
pub fn has_marker(text: &str, syntax: Syntax) -> bool {
    let inline = |l: &Line<'_>| {
        syntax.inline() && !matches!(inline_markers(l, &[]), Ok(found) if found.is_empty())
    };
    syntax.may_hold(text)
        && lines(text)
            .iter()
            .any(|l| is_marker_in(l.text, syntax) || inline(l))
}

/// `text`, one line, with the sums taken out of every closer inside it;
/// `None` when it holds none with sums.
fn strip_inline_sums(text: &str) -> Option<String> {
    let line = Line {
        number: 1,
        raw: text,
        text,
    };
    let found = inline_markers(&line, &[]).ok()?;
    let mut out = String::new();
    let mut at = 0;
    for m in &found {
        if !m.opener && !m.content.is_empty() {
            out.push_str(&text[at..m.start]);
            out.push_str(&rendered_closer(None, Comment::HTML));
            at = m.end;
        }
    }
    (at > 0).then(|| out + &text[at..])
}

/// `bytes`, the content of the file at `path`, with the sums taken out of
/// every closer line, so a snapshot of another template moves with its
/// prose and bodies and not with the sums the tool writes into it. A closer
/// is one in the file's own syntax or an HTML comment, which every file was
/// read for before files had syntaxes, and in Markdown and HTML also one
/// inside a line. Lines that are not UTF-8 are kept as they are.
pub fn strip_sums<'b>(path: &Path, bytes: &'b [u8]) -> std::borrow::Cow<'b, [u8]> {
    const NEEDLE: &[u8] = b"/computed";
    if !bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
        return std::borrow::Cow::Borrowed(bytes);
    }
    let own = Syntax::for_path(path);
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
        let closer =
            [own, Syntax::Markdown]
                .into_iter()
                .find_map(|syntax| match classify(&line, syntax) {
                    Ok(Kind::Closer {
                        indent,
                        content,
                        comment,
                    }) if !content.is_empty() => Some((indent, comment)),
                    _ => None,
                });
        let inline = || own.inline().then(|| strip_inline_sums(line.text)).flatten();
        match closer {
            Some((indent, comment)) => {
                out.extend_from_slice(indent.as_bytes());
                out.extend_from_slice(rendered_closer(None, comment).as_bytes());
                out.extend_from_slice(term);
            }
            None => match inline() {
                Some(stripped) => {
                    out.extend_from_slice(stripped.as_bytes());
                    out.extend_from_slice(term);
                }
                None => out.extend_from_slice(raw),
            },
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Parses a Markdown file into prose and regions. Every grammar error is
/// tier 2.
pub fn parse(text: &str) -> Result<File, ParseError> {
    parse_as(text, Syntax::Markdown)
}

/// Parses a file of `syntax` into prose and regions. Every grammar error
/// is tier 2.
pub fn parse_as(text: &str, syntax: Syntax) -> Result<File, ParseError> {
    let lines = lines(text);
    let mut segments = Vec::new();
    let mut prose = String::new();
    let fenced = fenced_in(&lines, syntax);
    let code = code_spans(&lines, &fenced, syntax);
    let mut names: Vec<String> = Vec::new();
    let mut named = |name: &Option<String>, line: usize| {
        if let Some(name) = name {
            if names.contains(name) {
                return Err(error(line, format!("duplicate name {name:?}")));
            }
            names.push(name.clone());
        }
        Ok(())
    };
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if fenced[i] {
            prose.push_str(line.raw);
            i += 1;
            continue;
        }
        match classify(line, syntax)? {
            Kind::Prose if syntax.inline() && line.text.contains("<!--") => {
                let found = inline_markers(line, &code[i])?;
                let mut at = 0;
                for pair in found.chunks(2) {
                    let (o, c) = match pair {
                        [o, c] if o.opener && !c.opener => (o, c),
                        [o, ..] if !o.opener => {
                            return Err(error(line.number, "closer without opener"));
                        }
                        [_, _] => {
                            return Err(error(
                                line.number,
                                "opener inside a body: nesting is not supported",
                            ));
                        }
                        _ => {
                            return Err(error(
                                line.number,
                                "a region inside a line needs its closer later on the same line",
                            ));
                        }
                    };
                    let opener = parse_opener(line.number, o.content)?
                        .placed(syntax, true)
                        .map_err(|m| error(line.number, m))?;
                    named(&opener.name, line.number)?;
                    prose.push_str(&line.text[at..o.start]);
                    if !prose.is_empty() {
                        segments.push(Segment::Prose(std::mem::take(&mut prose)));
                    }
                    segments.push(Segment::Region(Region {
                        line: line.number,
                        indent: String::new(),
                        raw_opener: line.text[o.start..o.end].to_string(),
                        raw_closer: line.text[c.start..c.end].to_string(),
                        body: line.text[o.end..c.start].to_string(),
                        sums: parse_closer(line.number, c.content)?,
                        opener,
                        syntax,
                        comment: Comment::HTML,
                        column: Some(line.text[..o.start].chars().count() + 1),
                    }));
                    at = c.end;
                }
                prose.push_str(&line.raw[at..]);
                i += 1;
            }
            Kind::Prose => {
                prose.push_str(line.raw);
                i += 1;
            }
            Kind::Closer { .. } => return Err(error(line.number, "closer without opener")),
            Kind::Opener {
                indent,
                content,
                comment,
            } => {
                let opener = parse_opener(line.number, content)?
                    .placed(syntax, false)
                    .map_err(|m| error(line.number, m))?;
                named(&opener.name, line.number)?;
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
                    match classify(l, syntax)? {
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
                    syntax,
                    comment,
                    column: None,
                }));
                i = j + 1;
            }
        }
    }
    if !prose.is_empty() {
        segments.push(Segment::Prose(prose));
    }
    Ok(File { segments, syntax })
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
    let (default_sink, default_lang) = defaults(&loader, &attrs);
    if !seen.contains(&"as") {
        sink = default_sink.unwrap_or(sink);
    }
    if !seen.contains(&"lang") {
        lang = default_lang.to_string();
    }
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
        recipe: None,
        tokens,
    };
    validate(line, &opener)?;
    Ok(opener)
}

/// The sink and language a loader defaults to when its choice depends on
/// its attributes: `symbol` fences code in its source's language and shows
/// a doc comment as Markdown; a `transcript` is a console session.
fn defaults(loader: &str, attrs: &[(String, String)]) -> (Option<Sink>, &'static str) {
    let attr = |key: &str| {
        attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    match loader {
        "symbol" if attr("part") == Some("doc") => (Some(Sink::Raw), ""),
        "symbol" => (None, attr("src").map_or("", crate::symbol::fence_lang)),
        "transcript" => (None, "console"),
        _ => (None, ""),
    }
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
            if opener.flag("sandbox") && opener.flag("volatile") {
                return Err(error(
                    line,
                    "sandbox needs inputs=: the sandbox allows reading only the declared inputs",
                ));
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
        "symbol" => crate::symbol::validate(opener).map_err(|m| error(line, m)),
        "git" => crate::git::validate(opener).map_err(|m| error(line, m)),
        "remote" => crate::remote::validate(opener).map_err(|m| error(line, m)),
        "transcript" => crate::transcript::validate(opener).map_err(|m| error(line, m)),
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
    fn sandbox_is_an_exec_flag_that_needs_inputs() {
        let f =
            parse("<!-- computed exec cmd=x inputs=a sandbox -->\n<!-- /computed -->\n").unwrap();
        assert!(region(&f, 0).opener.flag("sandbox"));
        assert!(region(&f, 0).opener.canonical().contains(" sandbox "));
        let e = err("x\n<!-- computed exec cmd=x volatile sandbox -->\n<!-- /computed -->\n");
        assert_eq!(e.line, 2);
        assert!(e.message.contains("sandbox needs inputs="), "{}", e.message);
        let e = err("<!-- computed tree sandbox -->\n<!-- /computed -->\n");
        assert!(e.message.contains("unknown flag"), "{}", e.message);
    }

    #[test]
    fn with_attr_replaces_a_value_in_place_or_appends_it() {
        let f = parse("<!-- computed exec cmd=\"cat a\" inputs=a name=n -->\n<!-- /computed -->\n")
            .unwrap();
        let o = &region(&f, 0).opener;
        assert_eq!(
            o.with_attr("inputs", "a,b c").canonical(),
            "<!-- computed exec cmd=\"cat a\" inputs=\"a,b c\" name=n -->"
        );
        assert_eq!(o.with_attr("inputs", "b").attr("inputs"), Some("b"));
        assert_eq!(
            o.with_attr("timeout", "5").canonical(),
            "<!-- computed exec cmd=\"cat a\" inputs=a name=n timeout=5 -->"
        );
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
        let md = Syntax::Markdown;
        assert_eq!(
            unfenced_marker_line("```\n<!-- /computed -->\n```\n", md),
            None
        );
        assert_eq!(
            unfenced_marker_line("x\n  <!-- /computed -->\n", md),
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
            &*strip_sums(Path::new("a.md"), &bytes),
            b"a /computed b\r\n  <!-- /computed -->\r\n<!-- /computed -->\n\xff"
        );
        assert!(matches!(
            strip_sums(Path::new("a.md"), b"no markers"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn a_lookalike_comment_is_prose() {
        let text = "<!-- computedx tree -->\n<!--computed tree-->\n<!-- Computed tree -->\n";
        let file = parse(text).unwrap();
        assert_eq!(file.segments.len(), 1);
    }

    fn regions_in(text: &str, syntax: Syntax) -> Vec<Region> {
        let file = parse_as(text, syntax).unwrap();
        assert_eq!(serialise(&file), text);
        file.segments
            .into_iter()
            .filter_map(|s| match s {
                Segment::Region(r) => Some(r),
                Segment::Prose(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_syntax_is_chosen_by_name_else_extension() {
        let of = |p: &str| Syntax::of(Path::new(p));
        assert_eq!(of("src/lib.rs"), Some(Syntax::Slash));
        assert_eq!(of("a/b.PY"), Some(Syntax::Hash));
        assert_eq!(of("Makefile"), Some(Syntax::Hash));
        assert_eq!(of(".gitignore"), Some(Syntax::Hash));
        assert_eq!(of("q.sql"), Some(Syntax::Dash));
        assert_eq!(of("site.css"), Some(Syntax::Css));
        assert_eq!(of("index.html"), Some(Syntax::Html));
        assert_eq!(of("README.md"), Some(Syntax::Markdown));
        assert_eq!(of("notes.txt"), None);
        assert_eq!(Syntax::for_path(Path::new("notes.txt")), Syntax::Markdown);
    }

    #[test]
    fn line_comment_markers_round_trip_in_each_leader() {
        let text = "fn a() {}\n    // computed tree src=. depth=1\n    // x\n    // /computed\n//! computed tree name=d\n//! /computed\n/// computed exec cmd=x volatile\n/// /computed\n";
        let rs = regions_in(text, Syntax::Slash);
        assert_eq!(rs.len(), 3);
        assert_eq!((rs[0].line, rs[0].indent.as_str()), (2, "    "));
        assert_eq!(rs[0].comment.open, "//");
        assert_eq!(rs[0].body, "    // x\n");
        assert_eq!(rs[1].comment.open, "//!");
        assert_eq!(rs[2].comment.open, "///");
        assert_eq!(
            rs[0].opener.canonical(),
            "<!-- computed tree src=. depth=1 -->"
        );
        let sql = regions_in("-- computed file src=a.sql\n-- /computed\n", Syntax::Dash);
        assert_eq!(sql[0].comment.open, "--");
        let css = regions_in(
            "a {}\n/* computed file src=a.css */\n/* /computed */\n",
            Syntax::Css,
        );
        assert_eq!(css[0].comment, Comment::CSS);
        let py = regions_in(
            "#!/bin/sh\n# computed exec cmd=x volatile | do not edit; run computed\n# /computed in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715\n",
            Syntax::Hash,
        );
        assert!(py[0].sums.is_some());
    }

    #[test]
    fn a_line_comment_opens_only_with_a_known_loader() {
        let text = "// computed values are cached\n// computed\n// computedtree\n//// computed tree\n//computed tree\n## computed tree\n// /computedx\n";
        assert!(regions_in(text, Syntax::Slash).is_empty());
        assert!(
            regions_in(
                "# computed, then rendered\n#! computed tree\n",
                Syntax::Hash
            )
            .is_empty()
        );
        // A known loader followed by prose is a marker that does not parse.
        let e = parse_as(
            "// computed tree is the loader\n// /computed\n",
            Syntax::Slash,
        )
        .unwrap_err();
        assert!(e.message.contains("unknown flag \"is\""), "{}", e.message);
        let e = parse_as("x\n# /computed\n", Syntax::Hash).unwrap_err();
        assert_eq!((e.line, e.message.as_str()), (2, "closer without opener"));
        // Another syntax's markers are prose.
        assert!(
            regions_in(
                "<!-- computed tree -->\n<!-- /computed -->\n",
                Syntax::Slash
            )
            .is_empty()
        );
        assert!(regions_in("// computed tree\n// /computed\n", Syntax::Markdown).is_empty());
    }

    #[test]
    fn only_markdown_has_fences() {
        let text = "```\n// computed tree\n// /computed\n```\n";
        assert_eq!(regions_in(text, Syntax::Slash).len(), 1);
        let html = "<pre>\n```\n<!-- computed tree -->\n<!-- /computed -->\n```\n</pre>\n";
        assert_eq!(regions_in(html, Syntax::Html).len(), 1);
        assert!(regions_in(html, Syntax::Markdown).is_empty());
    }

    #[test]
    fn a_css_marker_ends_with_its_comment() {
        let e = parse_as("/* computed tree\n/* /computed */\n", Syntax::Css).unwrap_err();
        assert!(e.message.contains("does not end with */"), "{}", e.message);
        let e = parse_as(
            "/* computed exec cmd=\"a */ b\" volatile */\n/* /computed */\n",
            Syntax::Css,
        )
        .unwrap_err();
        assert!(e.message.contains("holds */"), "{}", e.message);
    }

    #[test]
    fn outside_markdown_a_fence_default_is_raw_and_as_comment_needs_a_line_leader() {
        let sink = |text: &str, syntax| regions_in(text, syntax)[0].opener.sink;
        assert_eq!(
            sink("// computed tree\n// /computed\n", Syntax::Slash),
            Sink::Raw
        );
        assert_eq!(
            sink("<!-- computed tree -->\n<!-- /computed -->\n", Syntax::Html),
            Sink::Raw
        );
        assert_eq!(
            sink(
                "<!-- computed tree -->\n<!-- /computed -->\n",
                Syntax::Markdown
            ),
            Sink::Fence
        );
        assert_eq!(
            sink("// computed tree as=fence\n// /computed\n", Syntax::Slash),
            Sink::Fence
        );
        assert_eq!(
            sink("# computed tree as=comment\n# /computed\n", Syntax::Hash),
            Sink::Comment
        );
        for (text, syntax) in [
            (
                "<!-- computed tree as=comment -->\n<!-- /computed -->\n",
                Syntax::Markdown,
            ),
            (
                "/* computed tree as=comment */\n/* /computed */\n",
                Syntax::Css,
            ),
        ] {
            let e = parse_as(text, syntax).unwrap_err();
            assert!(
                e.message.contains("as=comment needs a line comment"),
                "{}",
                e.message
            );
        }
    }

    #[test]
    fn markers_are_written_in_the_openers_comment() {
        let r = &regions_in("//! computed tree src=.\n// /computed\n", Syntax::Slash)[0];
        assert_eq!(
            rendered_opener(&r.opener, r.comment),
            "//! computed tree src=. | do not edit; run computed"
        );
        assert_eq!(rendered_closer(None, r.comment), "//! /computed");
        assert_eq!(
            opener_line(&r.opener, Comment::CSS),
            "/* computed tree src=. */"
        );
    }

    #[test]
    fn strip_sums_reads_closers_in_the_files_syntax_and_html_everywhere() {
        let sums = "in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715";
        let text = format!("  // /computed {sums}\n<!-- /computed {sums} -->\n");
        assert_eq!(
            &*strip_sums(Path::new("lib.rs"), text.as_bytes()),
            b"  // /computed\n<!-- /computed -->\n"
        );
        let md = strip_sums(Path::new("a.md"), text.as_bytes()).into_owned();
        assert_eq!(
            String::from_utf8(md).unwrap(),
            format!("  // /computed {sums}\n<!-- /computed -->\n")
        );
    }

    const SUMS: &str = "in=9f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f609f3a1c0b7d2e4f60 out=41c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f71541c0d9e8b3a2f715";

    #[test]
    fn a_region_inside_a_line_holds_the_text_between_its_markers() {
        let text = format!(
            "# Title\n\nThe release is <!-- computed value src=Cargo.toml key=package.version name=v -->0.2.0<!-- /computed {SUMS} -->, and\ncafé <!-- computed value src=a.json key=b -->x<!-- /computed --> <!--  computed value src=a.json key=c-->y<!-- /computed-->\r\n"
        );
        let file = parse(&text).unwrap();
        assert_eq!(serialise(&file), text);
        let rs = regions_in(&text, Syntax::Markdown);
        assert_eq!(rs.len(), 3);
        assert_eq!((rs[0].line, rs[0].column), (3, Some(16)));
        assert_eq!(
            rs[0].raw_opener,
            "<!-- computed value src=Cargo.toml key=package.version name=v -->"
        );
        assert_eq!(rs[0].body, "0.2.0");
        assert_eq!(rs[0].raw_closer, format!("<!-- /computed {SUMS} -->"));
        assert!(rs[0].sums.is_some());
        assert_eq!(
            (rs[1].line, rs[1].column, rs[1].body.as_str()),
            (4, Some(6), "x")
        );
        assert_eq!((rs[2].column, rs[2].body.as_str()), (Some(66), "y"));
        assert_eq!(
            rs[2].opener.canonical(),
            "<!-- computed value src=a.json key=c -->"
        );
        assert_eq!(rs[0].last_line(), 3);
        assert_eq!(rs[0].place(), "3:16");
        assert!(matches!(file.segments.last(), Some(Segment::Prose(p)) if p == "\r\n"));
        // A loader that fences by default writes raw inside a line.
        let r = &regions_in(
            "a <!-- computed tree -->.<!-- /computed --> b\n",
            Syntax::Markdown,
        )[0];
        assert_eq!(r.opener.sink, Sink::Raw);
    }

    #[test]
    fn inline_markers_in_code_spans_and_fences_are_prose() {
        let prose = [
            "`<!-- computed value src=a key=b -->x<!-- /computed -->`\n",
            "`` a ` <!-- computed value src=a key=b -->x<!-- /computed --> ``\n",
            "a `code\nstill <!-- /computed --> code` b\n",
            "a `code\n<!-- computed tree -->x<!-- /computed --> code` b\n",
            "```\na <!-- computed tree --> b\n```\n",
            "| a | `<!-- computed tree src=. -->` |\n",
        ];
        for text in prose {
            assert!(regions_in(text, Syntax::Markdown).is_empty(), "{text:?}");
        }
        // A backtick that closes nothing masks nothing, and a heading ends
        // the paragraph a span could run on in.
        let rs = regions_in(
            "a ` b <!-- computed tree -->x<!-- /computed -->\n# a `\nb <!-- computed tree -->y<!-- /computed --> `\n",
            Syntax::Markdown,
        );
        assert_eq!(rs.len(), 2);
        // Backticks inside a marker open no span.
        let rs = regions_in(
            "<!-- computed exec cmd=\"echo `date`\" volatile -->x<!-- /computed --> `\n",
            Syntax::Markdown,
        );
        assert_eq!(rs.len(), 1);
    }

    #[test]
    fn inline_regions_are_markdown_and_html_only_and_html_scripts_hold_none() {
        let html = "<p>v<!-- computed value src=a key=b -->1<!-- /computed --></p>\n<script>\nx = '<!-- computed tree -->';\n</script>\n";
        assert_eq!(regions_in(html, Syntax::Html).len(), 1);
        assert!(
            regions_in(
                "<script>\n<!-- computed tree -->\n<!-- /computed -->\n</script>\n",
                Syntax::Html
            )
            .is_empty()
        );
        let e = parse_as("/* computed tree */ x\n/* /computed */\n", Syntax::Css).unwrap_err();
        assert!(
            e.message.contains("a marker is a whole line"),
            "{}",
            e.message
        );
        assert!(
            regions_in(
                "// a <!-- computed tree -->x<!-- /computed -->\n",
                Syntax::Slash
            )
            .is_empty()
        );
    }

    #[test]
    fn every_inline_grammar_error_is_reported_with_its_line() {
        let cases: &[(&str, usize, &str)] = &[
            (
                "a\nb <!-- computed tree --> c\n",
                2,
                "closer later on the same line",
            ),
            ("a <!-- /computed --> b\n", 1, "closer without opener"),
            (
                "a <!-- computed tree -->x<!-- /computed -->y<!-- /computed -->\n",
                1,
                "closer without opener",
            ),
            (
                "a <!-- computed tree --><!-- computed tree -->x<!-- /computed -->\n",
                1,
                "nesting is not supported",
            ),
            (
                "a <!-- computed tree src=.\n",
                1,
                "does not end on its line",
            ),
            (
                "<!-- computed tree --> trailing\n",
                1,
                "closer later on the same line",
            ),
            (
                "a <!-- computed tree as=fence -->x<!-- /computed -->\n",
                1,
                "as=fence",
            ),
            (
                "a <!-- computed tree max-lines=1 -->x<!-- /computed -->\n",
                1,
                "max-lines=",
            ),
            (
                "a <!-- computed nope -->x<!-- /computed -->\n",
                1,
                "unknown loader",
            ),
            (
                "<!-- computed tree name=a -->\n<!-- /computed -->\nx <!-- computed tree name=a -->y<!-- /computed -->\n",
                3,
                "duplicate name",
            ),
            (
                "a <!-- computed tree -->x<!-- /computed in=9f3a -->\n",
                1,
                "malformed sum",
            ),
        ];
        for (text, line, needle) in cases {
            let e = err(text);
            assert_eq!(e.line, *line, "line for {text:?}: {}", e.message);
            assert!(e.message.contains(needle), "{text:?}: {:?}", e.message);
        }
        assert!(
            regions_in(
                "a <!-- computed tree as=raw -->x<!-- /computed -->\n",
                Syntax::Markdown
            )
            .len()
                == 1
        );
    }

    #[test]
    fn strip_sums_and_has_marker_see_inside_a_line() {
        let text = format!("v <!-- computed tree -->x<!-- /computed {SUMS} --> y\n");
        assert_eq!(
            &*strip_sums(Path::new("a.md"), text.as_bytes()),
            b"v <!-- computed tree -->x<!-- /computed --> y\n"
        );
        assert_eq!(
            &*strip_sums(Path::new("a.rs"), text.as_bytes()),
            text.as_bytes()
        );
        assert!(has_marker(&text, Syntax::Markdown));
        assert!(!has_marker("a `<!-- x -->` b\n", Syntax::Markdown));
    }
}
