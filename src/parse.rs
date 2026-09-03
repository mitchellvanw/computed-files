use std::collections::BTreeMap;

pub struct Region {
    pub indent: String,
    pub open_line: String,
    pub loader: String,
    pub attrs: BTreeMap<String, String>,
    pub name: String,
    pub body: String,
    pub in_sum: Option<String>,
    pub out_sum: Option<String>,
}

pub enum Seg {
    Prose(String),
    Region(Region),
    Error { line: String, message: String },
}

pub fn parse(text: &str) -> Vec<Seg> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut segs = Vec::new();
    let mut prose: Vec<&str> = Vec::new();
    let mut n = 0usize;
    let mut i = 0usize;

    fn flush(prose: &mut Vec<&str>, segs: &mut Vec<Seg>) {
        if !prose.is_empty() {
            segs.push(Seg::Prose(prose.join("\n")));
            prose.clear();
        }
    }

    while i < lines.len() {
        match opener(lines[i]) {
            None => {
                prose.push(lines[i]);
                i += 1;
            }
            Some((loader, attrs)) => {
                let indent: String = lines[i].chars().take_while(|c| c.is_whitespace()).collect();
                let open_line = lines[i].trim().to_string();
                let mut j = i + 1;
                let mut body: Vec<&str> = Vec::new();
                while j < lines.len() && closer(lines[j]).is_none() {
                    body.push(lines[j]);
                    j += 1;
                }
                if j >= lines.len() {
                    flush(&mut prose, &mut segs);
                    segs.push(Seg::Error {
                        line: open_line,
                        message: format!("region opened at line {} has no closing marker", i + 1),
                    });
                    break;
                }
                let (in_sum, out_sum) = closer(lines[j]).unwrap();
                flush(&mut prose, &mut segs);
                n += 1;
                let name = attrs
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| format!("{}#{}", loader, n));
                segs.push(Seg::Region(Region {
                    indent,
                    open_line,
                    loader,
                    attrs,
                    name,
                    body: body.join("\n"),
                    in_sum,
                    out_sum,
                }));
                i = j + 1;
            }
        }
    }
    flush(&mut prose, &mut segs);
    segs
}

fn comment_inner(line: &str) -> Option<&str> {
    line.trim().strip_prefix("<!--")?.strip_suffix("-->")
}

fn opener(line: &str) -> Option<(String, BTreeMap<String, String>)> {
    let inner = comment_inner(line)?.trim();
    let mut parts = inner.splitn(3, char::is_whitespace);
    if parts.next()? != "computed" {
        return None;
    }
    let loader = parts.next()?;
    if loader.is_empty() || !loader.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let attrs = parse_attrs(parts.next().unwrap_or(""));
    Some((loader.to_string(), attrs))
}

fn closer(line: &str) -> Option<(Option<String>, Option<String>)> {
    let inner = comment_inner(line)?.trim();
    let rest = inner.strip_prefix("/computed")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let attrs = parse_attrs(rest);
    Some((attrs.get("sum").cloned(), attrs.get("out").cloned()))
}

fn parse_attrs(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < b.len() {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        let ks = i;
        while i < b.len() && b[i] != '=' && !b[i].is_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != '=' {
            while i < b.len() && !b[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        let key: String = b[ks..i].iter().collect();
        i += 1;
        let mut val = String::new();
        if i < b.len() && b[i] == '"' {
            i += 1;
            while i < b.len() && b[i] != '"' {
                val.push(b[i]);
                i += 1;
            }
            if i < b.len() {
                i += 1;
            }
        } else {
            while i < b.len() && !b[i].is_whitespace() {
                val.push(b[i]);
                i += 1;
            }
        }
        out.insert(key, val);
    }
    out
}
