//! `max-lines=N`: loader text longer than N lines is cut, after
//! normalisation and before the sink shapes it, to its first N lines and one
//! note that says how many were dropped. Pure: text in, text out.

use std::borrow::Cow;

use crate::marker::{Opener, Sink};
use crate::sink;

/// The line that stands for the `dropped` lines a cut left out. It cannot
/// parse as a marker or open a fence, so it is safe in a `raw` body.
pub fn note(dropped: usize) -> String {
    match dropped {
        1 => "… 1 more line".to_string(),
        n => format!("… {n} more lines"),
    }
}

/// The loader text the sink shapes: as it came when the opener has no
/// `max-lines=`, else normalised and cut. An error is a loader failure, as
/// the sink's own normalisation would report it.
///
/// A cut can leave a fence in `raw` text unclosed; the sink's parse-back
/// check then fails the region as it fails any unbalanced text.
pub fn apply<'t>(opener: &Opener, text: &'t str) -> Result<Cow<'t, str>, String> {
    let Some(max) = opener.max_lines else {
        return Ok(Cow::Borrowed(text));
    };
    let text = sink::normalise(text.as_bytes())?;
    Ok(Cow::Owned(cut(opener.sink, &text, max)))
}

/// Normalised `text` cut to its first `max` lines, with the note where the
/// sink holds text line for line: as the last line, so `raw` shows it after
/// the text and `fence` inside the fence.
///
/// This is the hook for a sink that parses its text rather than holding it,
/// such as a table: its arm must keep the note out of what it parses and
/// place it after what it shapes.
fn cut(sink: Sink, text: &str, max: usize) -> String {
    let lines = if text.is_empty() {
        0
    } else {
        text.split('\n').count()
    };
    if lines <= max {
        return text.to_string();
    }
    let kept: Vec<&str> = text.split('\n').take(max).collect();
    let note = note(lines - max);
    match sink {
        Sink::Raw | Sink::Fence => format!("{}\n{note}", kept.join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_counts_the_lines_dropped() {
        assert_eq!(note(1), "… 1 more line");
        assert_eq!(note(40), "… 40 more lines");
        assert!(!crate::marker::is_marker(&note(3)));
        assert!(!crate::marker::has_unclosed_fence(&note(3)));
    }

    #[test]
    fn cut_keeps_the_first_lines_and_counts_the_rest() {
        assert_eq!(cut(Sink::Raw, "a\nb\nc", 1), "a\n… 2 more lines");
        assert_eq!(cut(Sink::Raw, "a\n\nc", 2), "a\n\n… 1 more line");
        assert_eq!(cut(Sink::Fence, "a\nb", 2), "a\nb");
        assert_eq!(cut(Sink::Raw, "", 1), "");
    }
}
