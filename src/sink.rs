//! The two sinks, `raw` and `fence`, and the normalisation every loader's
//! text goes through before a sink shapes it. Pure: text in, body out.

use crate::marker::{self, Sink};

/// Normalises loader output before a sink shapes it: invalid UTF-8, a C0
/// control other than tab, LF and CR, or a line that would parse as a marker
/// is a failure; CRLF and lone CR become LF; trailing newlines are stripped.
pub fn normalise(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("output is not UTF-8: {e}"))?;
    if let Some(c) = text.chars().find(|&c| c.is_control() && c != '\t' && c != '\n' && c != '\r' && (c as u32) < 0x20) {
        return Err(format!("output contains the control byte {:#04x}", c as u32));
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim_end_matches('\n');
    if let Some(line) = text.lines().find(|l| marker::is_marker(l)) {
        return Err(format!("output contains a line that would parse as a marker: {line}"));
    }
    Ok(text.to_string())
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
        .map(|l| l.trim_start_matches(' ').chars().take_while(|&c| c == '`').count())
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

/// Normalises, shapes with the sink, and checks the body parses back to
/// itself between markers. An error is a loader failure.
pub fn body(sink: Sink, lang: &str, bytes: &[u8]) -> Result<String, String> {
    let text = normalise(bytes)?;
    let body = match sink {
        Sink::Raw => raw(&text),
        Sink::Fence => fence(&text, lang),
    };
    let probe = format!("<!-- computed exec cmd=x volatile -->\n{body}<!-- /computed -->\n");
    match marker::parse(&probe) {
        Ok(file) => match file.segments.as_slice() {
            [marker::Segment::Region(r)] if r.body == body => Ok(body),
            _ => Err("output does not parse back to itself inside a region".to_string()),
        },
        Err(_) => Err("output has an unbalanced fence that would swallow the closer".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::Sink;

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
            (b"x\n<!-- computed tree -->\n", Err("marker")),
            (b"x\n  <!-- /computed -->\n", Err("marker")),
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
        assert_eq!(fence("```rust\nx\n```", ""), "````\n```rust\nx\n```\n````\n");
        assert_eq!(fence("   `````\ny", "md"), "``````md\n   `````\ny\n``````\n");
        assert_eq!(fence("a ``` b", ""), "```\na ``` b\n```\n");
    }

    #[test]
    fn body_shapes_and_parses_back() {
        assert_eq!(body(Sink::Fence, "", b".\n").unwrap(), "```\n.\n```\n");
        assert_eq!(body(Sink::Raw, "", b"| a |\n|---|\n").unwrap(), "\n| a |\n|---|\n\n");
        assert_eq!(body(Sink::Raw, "", b"```\ncode\n```\n").unwrap(), "\n```\ncode\n```\n\n");
    }

    #[test]
    fn raw_text_with_an_unbalanced_fence_is_a_loader_failure() {
        let e = body(Sink::Raw, "", b"```\nnever closed\n").unwrap_err();
        assert!(e.contains("fence"), "{e}");
        assert!(body(Sink::Fence, "", b"```\nnever closed\n").is_ok());
    }
}
