//! The sinks, `raw`, `fence`, `comment` and `table`, and the normalisation
//! every loader's text goes through before a sink shapes it. Pure: text in,
//! body out.

use crate::marker::{self, Comment, Opener, Region, Sink};
use crate::table;
use crate::truncate;

/// Normalises loader output before a sink shapes it: invalid UTF-8 or a C0
/// control other than tab, LF and CR is a failure; CRLF and lone CR become
/// LF; trailing newlines are stripped.
pub fn normalise(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("output is not UTF-8: {e}"))?;
    if let Some(c) = text
        .chars()
        .find(|&c| c.is_control() && c != '\t' && c != '\n' && c != '\r' && (c as u32) < 0x20)
    {
        return Err(format!(
            "output contains the control byte {:#04x}",
            c as u32
        ));
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(text.trim_end_matches('\n').to_string())
}

/// A blank line, the text, a blank line.
pub fn raw(text: &str) -> String {
    let mut out = String::from("\n");
    if !text.is_empty() {
        out.push_str(text);
        out.push('\n');
    }
    out.push('\n');
    out
}

/// A backtick fence one longer than any backtick run starting a line of the
/// text, minimum three, with `lang` on the opening fence.
pub fn fence(text: &str, lang: &str) -> String {
    let longest = text
        .lines()
        .map(|l| {
            l.trim_start_matches(' ')
                .chars()
                .take_while(|&c| c == '`')
                .count()
        })
        .max()
        .unwrap_or(0);
    let run = "`".repeat((longest + 1).max(3));
    let mut out = format!("{run}{lang}\n");
    if !text.is_empty() {
        out.push_str(text);
        out.push('\n');
    }
    out.push_str(&run);
    out.push('\n');
    out
}

/// Every line of the text behind `comment`'s leader and a space, a blank
/// line behind the leader alone. With `lang`, the text is fenced first, so
/// a Rust doc comment holds a code block.
pub fn comment(text: &str, lang: &str, comment: Comment) -> String {
    let fenced;
    let text = if lang.is_empty() {
        text
    } else {
        fenced = fence(text, lang);
        fenced.strip_suffix('\n').unwrap_or(&fenced)
    };
    let mut out = String::new();
    if text.is_empty() {
        return out;
    }
    for line in text.split('\n') {
        out.push_str(comment.open);
        if !line.is_empty() {
            out.push(' ');
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// An inline region's body: the normalised text itself, which must be one
/// line, and must not hold a comment that would parse as a marker.
fn inline(region: &Region, text: String) -> Result<String, String> {
    if text.contains('\n') {
        return Err(format!(
            "the text is {} lines, and a region inside a line holds one",
            text.split('\n').count()
        ));
    }
    let c = region.comment;
    let probe = format!(
        "{}{text}{}\n",
        c.wrap("computed exec cmd=x volatile"),
        c.wrap("/computed")
    );
    match marker::parse_as(&probe, region.syntax) {
        Ok(file) if matches!(file.segments.as_slice(), [marker::Segment::Region(r), _] if r.body == text) => {
            Ok(text)
        }
        _ => Err(format!(
            "the text would not parse back inside the line's markers: {text}"
        )),
    }
}

/// Normalises, cuts to `max-lines=`, shapes with the region's sink, and
/// checks the body parses back to itself between markers in the region's
/// comment. An error is a loader failure. A line that would parse as a
/// marker fails a `raw` body unless a fence in the text holds it, and only
/// Markdown has fences; `fence` holds every line there, so marker examples
/// can be shown that way.
///
/// `raw`, `fence` and `comment` cut the text by lines, the note its last
/// line. `table` cuts its data rows, keeping the header, and puts the note
/// after the table as a paragraph of its own.
pub fn body(region: &Region, bytes: &[u8]) -> Result<String, String> {
    let Opener {
        sink,
        lang,
        max_lines,
        ..
    } = &region.opener;
    let syntax = region.syntax;
    let mut text = normalise(bytes)?;
    if region.column.is_some() {
        return inline(region, text);
    }
    if let (Some(max), Sink::Raw | Sink::Fence | Sink::Comment) = (max_lines, sink) {
        text = truncate::lines(&text, *max);
    }
    let body = match sink {
        Sink::Raw => {
            if let Some(line) = marker::unfenced_marker_line(&text, syntax) {
                return Err(format!(
                    "output contains a line that would parse as a marker: {line}"
                ));
            }
            if syntax.is_markdown() && marker::has_unclosed_fence(&text) {
                return Err(
                    "output has an unbalanced fence that would swallow the closer".to_string(),
                );
            }
            raw(&text)
        }
        Sink::Fence => fence(&text, lang),
        Sink::Comment => comment(&text, lang, region.comment),
        Sink::Table(from) => {
            let mut rows = table::rows(&text, *from)?;
            let note = max_lines.and_then(|max| truncate::rows(&mut rows, max));
            let mut body = raw(&table::shape(&rows));
            if let Some(note) = note {
                body.push_str(&note);
                body.push_str("\n\n");
            }
            body
        }
    };
    let c = region.comment;
    let probe = format!(
        "{}\n{body}{}\n",
        c.wrap("computed exec cmd=x volatile"),
        c.wrap("/computed")
    );
    let back = marker::parse_as(&probe, syntax);
    if let Ok(file) = &back
        && let [marker::Segment::Region(r)] = file.segments.as_slice()
        && r.body == body
    {
        return Ok(body);
    }
    if let Some(line) = marker::unfenced_marker_line(&body, syntax) {
        return Err(format!(
            "output contains a line that would parse as a marker: {line}"
        ));
    }
    Err(match back {
        Ok(_) => "output does not parse back to itself inside a region".to_string(),
        Err(_) => "output has an unbalanced fence that would swallow the closer".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::{Segment, Syntax};

    fn region(text: &str, syntax: Syntax) -> Region {
        match marker::parse_as(text, syntax).unwrap().segments.remove(0) {
            Segment::Region(r) => r,
            Segment::Prose(_) => unreachable!(),
        }
    }

    /// The body an exec region in Markdown with `attrs` makes of `bytes`.
    fn shaped(attrs: &str, bytes: &[u8]) -> Result<String, String> {
        let text = format!("<!-- computed exec cmd=x volatile {attrs} -->\n<!-- /computed -->\n");
        body(&region(&text, Syntax::Markdown), bytes)
    }

    #[test]
    fn normalisation_table() {
        let cases: &[(&[u8], Result<&str, &str>)] = &[
            (b"a\nb\n", Ok("a\nb")),
            (b"a\r\nb\r\n", Ok("a\nb")),
            (b"a\rb\r", Ok("a\nb")),
            (b"a\n\n\n", Ok("a")),
            (b"a  \nb\t\n", Ok("a  \nb\t")),
            (b"a\n\nb\n", Ok("a\n\nb")),
            (b"", Ok("")),
            (b"\n\n", Ok("")),
            (b"a\tb", Ok("a\tb")),
            (b"a\x00b", Err("control")),
            (b"a\x1bb", Err("control")),
            (b"\xff\xfe", Err("UTF-8")),
        ];
        for (input, expected) in cases {
            let got = normalise(input);
            match expected {
                Ok(text) => assert_eq!(got.as_deref(), Ok(*text), "{input:?}"),
                Err(needle) => {
                    let e = got.expect_err(&format!("{input:?} should fail"));
                    assert!(e.contains(needle), "{input:?}: {e}");
                }
            }
        }
    }

    #[test]
    fn raw_wraps_the_text_in_blank_lines() {
        assert_eq!(raw("a\nb"), "\na\nb\n\n");
        assert_eq!(raw(""), "\n\n");
    }

    #[test]
    fn fence_wraps_the_text_with_the_language() {
        assert_eq!(fence("a\nb", "text"), "```text\na\nb\n```\n");
        assert_eq!(fence("", ""), "```\n```\n");
    }

    #[test]
    fn fence_outruns_backtick_runs_in_the_text() {
        assert_eq!(
            fence("```rust\nx\n```", ""),
            "````\n```rust\nx\n```\n````\n"
        );
        assert_eq!(
            fence("   `````\ny", "md"),
            "``````md\n   `````\ny\n``````\n"
        );
        assert_eq!(fence("a ``` b", ""), "```\na ``` b\n```\n");
    }

    #[test]
    fn body_shapes_and_parses_back() {
        assert_eq!(shaped("as=fence", b".\n").unwrap(), "```\n.\n```\n");
        assert_eq!(shaped("", b"| a |\n|---|\n").unwrap(), "\n| a |\n|---|\n\n");
        assert_eq!(
            shaped("", b"```\ncode\n```\n").unwrap(),
            "\n```\ncode\n```\n\n"
        );
    }

    #[test]
    fn a_marker_line_fails_raw_unless_a_fence_holds_it() {
        for text in [
            &b"x\n<!-- computed tree -->\n"[..],
            b"x\n  <!-- /computed -->\n",
        ] {
            let e = shaped("", text).unwrap_err();
            assert!(e.contains("marker"), "{e}");
        }
        let example = b"```\n<!-- computed tree -->\n<!-- /computed -->\n```\n";
        assert!(shaped("", example).is_ok());
        assert_eq!(
            shaped(
                "as=fence lang=markdown",
                b"<!-- computed tree -->\n<!-- /computed -->\n"
            )
            .unwrap(),
            "```markdown\n<!-- computed tree -->\n<!-- /computed -->\n```\n"
        );
    }

    #[test]
    fn raw_text_with_an_unbalanced_fence_is_a_loader_failure() {
        let e = shaped("", b"```\nnever closed\n").unwrap_err();
        assert!(e.contains("fence"), "{e}");
        assert!(shaped("as=fence", b"```\nnever closed\n").is_ok());
    }

    #[test]
    fn comment_puts_the_leader_before_every_line() {
        assert_eq!(
            comment("a\n\nb", "", Comment::HTML),
            "<!-- a\n<!--\n<!-- b\n"
        );
        let r = region(
            "//! computed exec cmd=x volatile as=comment\n//! /computed\n",
            Syntax::Slash,
        );
        assert_eq!(body(&r, b".\n\nsrc\n").unwrap(), "//! .\n//!\n//! src\n");
        assert_eq!(body(&r, b"").unwrap(), "");
        let r = region(
            "# computed exec cmd=x volatile as=comment lang=text\n# /computed\n",
            Syntax::Hash,
        );
        assert_eq!(body(&r, b"a\n").unwrap(), "# ```text\n# a\n# ```\n");
    }

    #[test]
    fn a_marker_line_fails_in_the_regions_own_syntax() {
        let r = region(
            "// computed exec cmd=x volatile as=comment\n// /computed\n",
            Syntax::Slash,
        );
        let e = body(&r, b"computed tree\n").unwrap_err();
        assert!(e.contains("marker: // computed tree"), "{e}");
        assert!(body(&r, b"computed, as said\n").is_ok());
        let r = region(
            "# computed exec cmd=x volatile\n# /computed\n",
            Syntax::Hash,
        );
        assert!(body(&r, b"# /computed\n").is_err());
        // No fence holds a marker outside Markdown, and none needs closing.
        assert!(body(&r, b"```\n# /computed\n```\n").is_err());
        assert_eq!(body(&r, b"```\nopen\n").unwrap(), "\n```\nopen\n\n");
        assert_eq!(
            body(&r, b"<!-- /computed -->\n").unwrap(),
            "\n<!-- /computed -->\n\n"
        );
    }
}
