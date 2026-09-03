use crate::load::{Kind, Loaded};

pub fn render(name: &str, l: &Loaded) -> Result<String, String> {
    match name {
        "table" => Ok(table(l)),
        "fence" => Ok(fence(l)),
        "raw" => Ok(raw(l)),
        other => Err(format!("unknown sink \"{}\"", other)),
    }
}

pub fn table(l: &Loaded) -> String {
    let (header, rows) = match &l.kind {
        Kind::Rows { header, rows } => (header, rows),
        Kind::Text { .. } => return "| (no rows) |\n|---|".to_string(),
    };
    if rows.is_empty() {
        return "| (no rows) |\n|---|".to_string();
    }
    let w: Vec<usize> = header
        .iter()
        .enumerate()
        .map(|(i, h)| {
            h.chars()
                .count()
                .max(rows.iter().map(|r| r.get(i).map_or(0, |c| c.chars().count())).max().unwrap_or(0))
        })
        .collect();
    let line = |cells: &[String]| -> String {
        let mut s = String::from("| ");
        for (i, c) in cells.iter().enumerate() {
            if i > 0 {
                s.push_str(" | ");
            }
            s.push_str(c);
            s.push_str(&" ".repeat(w[i].saturating_sub(c.chars().count())));
        }
        s.push_str(" |");
        s
    };
    let sep = format!("|{}|", w.iter().map(|x| "-".repeat(x + 2)).collect::<Vec<_>>().join("|"));
    let mut out = vec![line(header), sep];
    for r in rows {
        out.push(line(r));
    }
    out.join("\n")
}

pub fn fence(l: &Loaded) -> String {
    let (lang, inner) = match &l.kind {
        Kind::Text { text, lang } => (lang.clone(), text.clone()),
        Kind::Rows { .. } => (String::new(), table(l)),
    };
    format!("```{}\n{}\n```", lang, inner)
}

pub fn raw(l: &Loaded) -> String {
    match &l.kind {
        Kind::Text { text, .. } => text.clone(),
        Kind::Rows { .. } => table(l),
    }
}
