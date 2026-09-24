//! `max-lines=N`: loader text longer than N lines is cut, after
//! normalisation, to its first N lines and one note that says how many were
//! dropped. A table is cut by its data rows instead, since its text is data.
//! The sink calls these as it shapes the body. Pure: text in, text out.

/// The line that stands for the `dropped` lines a cut left out. It cannot
/// parse as a marker or open a fence, so it is safe in a `raw` body.
pub fn note(dropped: usize) -> String {
    match dropped {
        1 => "… 1 more line".to_string(),
        n => format!("… {n} more lines"),
    }
}

/// Normalised `text` cut to its first `max` lines, with the note as the last
/// line, so `raw` shows it after the text and `fence` inside the fence.
///
/// A cut can leave a fence in `raw` text unclosed; the sink's parse-back
/// check then fails the region as it fails any unbalanced text.
pub fn lines(text: &str, max: usize) -> String {
    let lines = if text.is_empty() {
        0
    } else {
        text.split('\n').count()
    };
    if lines <= max {
        return text.to_string();
    }
    let kept: Vec<&str> = text.split('\n').take(max).collect();
    format!("{}\n{}", kept.join("\n"), note(lines - max))
}

/// A table's rows, header first, cut to the header and `max` data rows.
/// Each row is one line of the table, so the note counts the rows dropped
/// as lines. The note goes after the table, never into it; `None` when
/// nothing was cut.
pub fn rows<T>(rows: &mut Vec<T>, max: usize) -> Option<String> {
    let data = rows.len().saturating_sub(1);
    if data <= max {
        return None;
    }
    rows.truncate(max + 1);
    Some(note(data - max))
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
    fn lines_keeps_the_first_lines_and_counts_the_rest() {
        assert_eq!(lines("a\nb\nc", 1), "a\n… 2 more lines");
        assert_eq!(lines("a\n\nc", 2), "a\n\n… 1 more line");
        assert_eq!(lines("a\nb", 2), "a\nb");
        assert_eq!(lines("", 1), "");
    }

    #[test]
    fn rows_keeps_the_header_and_counts_the_data_rows_dropped() {
        let mut r = vec!["h", "1", "2", "3"];
        assert_eq!(rows(&mut r, 1), Some("… 2 more lines".to_string()));
        assert_eq!(r, ["h", "1"]);
        let mut r = vec!["h", "1", "2"];
        assert_eq!(rows(&mut r, 2), None);
        assert_eq!(r, ["h", "1", "2"]);
        let mut r = vec!["h"];
        assert_eq!(rows(&mut r, 1), None);
    }
}
