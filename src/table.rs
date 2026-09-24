//! The `table` sink: loader text that is CSV, TSV or JSON Lines, shaped
//! into a GitHub Markdown table with padded columns. Pure: text in, table
//! out. The first row is the header.

/// What the text a `table` sink shapes is written in, from `delim=` and
/// `from=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFrom {
    /// CSV with this delimiter: `,` by default, a tab with `delim=tab`.
    /// Quoted fields follow RFC 4180 for `,`; tab-separated text is read
    /// without quoting, as commands print it.
    Delimited(u8),
    /// One JSON object per line, `from=jsonl`. The first object's keys, in
    /// the order written, are the columns.
    Jsonl,
}

impl TableFrom {
    /// The format from the sink's own attributes, `delim=` and `from=`.
    pub fn parse(delim: Option<&str>, from: Option<&str>) -> Result<TableFrom, String> {
        match from {
            None | Some("csv") => match delim {
                None | Some("comma") => Ok(TableFrom::Delimited(b',')),
                Some("tab") => Ok(TableFrom::Delimited(b'\t')),
                Some(d) => Err(format!("delim={d}: expected comma or tab")),
            },
            Some("jsonl") if delim.is_some() => {
                Err("delim= does not apply to from=jsonl".to_string())
            }
            Some("jsonl") => Ok(TableFrom::Jsonl),
            Some(f) => Err(format!("from={f}: expected csv or jsonl")),
        }
    }
}

/// The table's lines, LF-separated, without a final newline. Text with no
/// header row, a row whose field count differs from the header's, or a
/// JSON line that is not an object is an error.
pub fn table(text: &str, from: TableFrom) -> Result<String, String> {
    let rows = match from {
        TableFrom::Delimited(delim) => delimited(text, delim)?,
        TableFrom::Jsonl => jsonl(text)?,
    };
    let Some(header) = rows.first() else {
        return Err("table: the text has no header row".to_string());
    };
    if let Some((i, row)) = rows
        .iter()
        .enumerate()
        .find(|(_, r)| r.len() != header.len())
    {
        return Err(format!(
            "table: row {} has {} field{}; the header has {}",
            i + 1,
            row.len(),
            if row.len() == 1 { "" } else { "s" },
            header.len()
        ));
    }
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|c| cell(c)).collect())
        .collect();
    let widths: Vec<usize> = (0..header.len())
        .map(|c| {
            rows.iter()
                .map(|r| r[c].chars().count())
                .max()
                .unwrap_or(0)
                .max(3)
        })
        .collect();
    let line = |cells: Vec<String>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(c, &w)| format!("{c}{}", " ".repeat(w - c.chars().count())))
            .collect();
        format!("| {} |", padded.join(" | "))
    };
    let mut out = vec![line(rows[0].clone())];
    out.push(line(widths.iter().map(|&w| "-".repeat(w)).collect()));
    out.extend(rows[1..].iter().cloned().map(line));
    Ok(out.join("\n"))
}

/// A field as a table cell. A cell is one line, so a newline inside a
/// quoted field becomes `<br>`; a `|` not already escaped is escaped, so
/// it cannot end the cell. The rest is Markdown and stays as it is.
fn cell(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut prev = None;
    for c in field.chars() {
        match c {
            '\n' => out.push_str("<br>"),
            '|' if prev != Some('\\') => out.push_str("\\|"),
            c => out.push(c),
        }
        prev = Some(c);
    }
    out
}

fn delimited(text: &str, delim: u8) -> Result<Vec<Vec<String>>, String> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delim)
        .quoting(delim != b'\t')
        .from_reader(text.as_bytes())
        .records()
        .map(|r| {
            r.map(|r| r.iter().map(str::to_string).collect())
                .map_err(|e| format!("table: {e}"))
        })
        .collect()
}

fn jsonl(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut columns: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let object = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(serde_json::Value::Object(o)) => o,
            Ok(_) => return Err(format!("table: line {} is not a JSON object", i + 1)),
            Err(e) => return Err(format!("table: line {}: {e}", i + 1)),
        };
        let columns = columns.get_or_insert_with(|| {
            let keys: Vec<String> = object.keys().cloned().collect();
            rows.push(keys.clone());
            keys
        });
        rows.push(
            columns
                .iter()
                .map(|k| match object.get(k) {
                    None | Some(serde_json::Value::Null) => String::new(),
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                })
                .collect(),
        );
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: TableFrom = TableFrom::Delimited(b',');

    #[test]
    fn csv_becomes_a_padded_table() {
        assert_eq!(
            table("name,version\ncomputed,0.2.0\nclap,4\n", CSV).unwrap(),
            "| name     | version |\n| -------- | ------- |\n| computed | 0.2.0   |\n| clap     | 4       |"
        );
    }

    #[test]
    fn columns_are_at_least_three_wide_and_a_header_alone_is_a_table() {
        assert_eq!(table("a,b", CSV).unwrap(), "| a   | b   |\n| --- | --- |");
    }

    #[test]
    fn quoted_fields_pipes_and_newlines() {
        assert_eq!(
            table(
                "k,v\n\"a, b\",\"x|y\"\n\"two\nlines\",\"already \\| escaped\"\n",
                CSV
            )
            .unwrap(),
            "| k            | v                  |\n| ------------ | ------------------ |\n| a, b         | x\\|y               |\n| two<br>lines | already \\| escaped |"
        );
    }

    #[test]
    fn tab_separated_text_is_read_without_quoting() {
        let from = TableFrom::parse(Some("tab"), None).unwrap();
        assert_eq!(
            table("a\tb\n\"q\"\tx,y\n", from).unwrap(),
            "| a   | b   |\n| --- | --- |\n| \"q\" | x,y |"
        );
    }

    #[test]
    fn json_lines_take_the_first_objects_keys_in_order() {
        let text = "{\"name\": \"b\", \"n\": 2, \"ok\": true}\n\n{\"n\": null, \"name\": \"a\", \"extra\": 1, \"ok\": [1]}\n{\"name\": \"c\"}\n";
        assert_eq!(
            table(text, TableFrom::Jsonl).unwrap(),
            "| name | n   | ok   |\n| ---- | --- | ---- |\n| b    | 2   | true |\n| a    |     | [1]  |\n| c    |     |      |"
        );
        let e = table("{\"a\": 1}\n[1]\n", TableFrom::Jsonl).unwrap_err();
        assert!(e.contains("line 2 is not a JSON object"), "{e}");
        let e = table("{\"a\": 1\n", TableFrom::Jsonl).unwrap_err();
        assert!(e.contains("line 1"), "{e}");
    }

    #[test]
    fn a_ragged_row_or_no_header_is_an_error() {
        let e = table("a,b\n1,2\n3\n", CSV).unwrap_err();
        assert_eq!(e, "table: row 3 has 1 field; the header has 2");
        let e = table("", CSV).unwrap_err();
        assert!(e.contains("no header row"), "{e}");
        assert!(table("", TableFrom::Jsonl).is_err());
    }

    #[test]
    fn the_attributes_parse() {
        assert_eq!(TableFrom::parse(None, None), Ok(CSV));
        assert_eq!(TableFrom::parse(Some("comma"), Some("csv")), Ok(CSV));
        assert_eq!(TableFrom::parse(None, Some("jsonl")), Ok(TableFrom::Jsonl));
        for (delim, from, needle) in [
            (Some(";"), None, "delim=;"),
            (None, Some("xml"), "from=xml"),
            (Some("tab"), Some("jsonl"), "does not apply"),
            (Some("comma"), Some("jsonl"), "does not apply"),
        ] {
            let e = TableFrom::parse(delim, from).unwrap_err();
            assert!(e.contains(needle), "{e}");
        }
    }
}
