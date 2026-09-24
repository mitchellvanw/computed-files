//! The `index` loader's text: one Markdown link per matched file, titled by
//! the file's first level-one heading or by its file name. Pure; the
//! loader expands the globs and reads the files.

use std::path::Path;

use crate::project;

/// Where a file's title comes from, `title=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Title {
    /// The text of the first `# ` heading of a Markdown file; the file name
    /// when there is none, and for any other file.
    H1,
    /// The file name, extension included.
    Filename,
}

impl Title {
    pub fn parse(s: &str) -> Option<Title> {
        match s {
            "h1" => Some(Title::H1),
            "filename" => Some(Title::Filename),
            _ => None,
        }
    }

    /// Whether this title reads the file at `rel`: only a heading of a
    /// Markdown file does.
    pub fn reads(self, rel: &Path) -> bool {
        self == Title::H1
            && rel
                .extension()
                .is_some_and(|e| e == "md" || e == "markdown")
    }
}

/// The title of the file at `rel`, from `content` when the title reads it.
/// The heading's text as written, inline Markdown kept; a file that is not
/// UTF-8 is read lossily. A template's heading is one of its prose, as a
/// reader sees it: a region inside the line is its body, and a heading in a
/// region body is not the file's.
pub fn title(rel: &Path, content: Option<&[u8]>) -> String {
    let heading = content.and_then(|c| {
        let text = String::from_utf8_lossy(c);
        crate::toc::prose_headings(&text)
            .unwrap_or_else(|_| project::headings(&text))
            .into_iter()
            .find(|h| h.level == 1 && !h.text.is_empty())
    });
    match heading {
        Some(h) => h.text,
        None => rel
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// One list item: `- [title](path)`.
pub fn line(title: &str, rel: &str) -> String {
    format!("- [{title}]({})", destination(rel))
}

/// A path as a link destination: the characters that would end or break
/// one are percent-encoded, and `%` itself so a name holding an escape
/// still names its file.
fn destination(rel: &str) -> String {
    let mut out = String::with_capacity(rel.len());
    for c in rel.chars() {
        match c {
            ' ' | '(' | ')' | '<' | '>' | '%' => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_is_the_first_h1_outside_front_matter_and_fences() {
        let rel = Path::new("docs/adr/0001-rust.md");
        let doc = b"---\nstatus: accepted\n# not this\n---\n\n```sh\n# nor this\n```\n\n## Not a level one\n\n# Rust for the `prototype`\n\n# A second\n";
        assert_eq!(title(rel, Some(doc)), "Rust for the `prototype`");
        assert_eq!(title(rel, Some(b"no heading\n")), "0001-rust.md");
        assert_eq!(title(rel, Some(b"#\n# Real\n")), "Real");
        assert_eq!(title(rel, None), "0001-rust.md");
    }

    #[test]
    fn a_template_s_title_is_its_prose_as_a_reader_sees_it() {
        let rel = Path::new("b.md");
        let inline = b"# B is <!-- computed value src=v.toml key=x -->one<!-- /computed -->\n";
        assert_eq!(title(rel, Some(inline)), "B is one");
        let body =
            b"<!-- computed file src=banner.md -->\n# Banner\n<!-- /computed -->\n\n# Real title\n";
        assert_eq!(title(rel, Some(body)), "Real title");
        // A file whose markers do not parse is read as it stands.
        assert_eq!(
            title(rel, Some(b"# T <!-- computed x\n")),
            "T <!-- computed x"
        );
    }

    #[test]
    fn only_markdown_is_read_for_a_heading() {
        assert!(Title::H1.reads(Path::new("a.md")));
        assert!(Title::H1.reads(Path::new("a.markdown")));
        assert!(!Title::H1.reads(Path::new("build.sh")));
        assert!(!Title::Filename.reads(Path::new("a.md")));
    }

    #[test]
    fn a_line_links_the_path_as_written() {
        assert_eq!(
            line("Two sums", "docs/adr/0002-two-sum-closer.md"),
            "- [Two sums](docs/adr/0002-two-sum-closer.md)"
        );
        assert_eq!(
            line("Notes", "../my notes (draft) 100%.md"),
            "- [Notes](../my%20notes%20%28draft%29%20100%25.md)"
        );
    }
}
