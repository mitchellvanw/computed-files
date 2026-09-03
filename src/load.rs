use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

pub enum Kind {
    Text { text: String, lang: String },
    Rows { header: Vec<String>, rows: Vec<Vec<String>> },
}

pub struct Loaded {
    pub snapshot: Option<String>,
    pub kind: Kind,
}

pub fn default_sink(loader: &str) -> &'static str {
    match loader {
        "tree" => "fence",
        "csv" => "table",
        "sh" => "raw",
        _ => "raw",
    }
}

fn attr(attrs: &BTreeMap<String, String>, key: &str, default: &str) -> String {
    attrs.get(key).cloned().unwrap_or_else(|| default.to_string())
}

pub fn tree(root: &Path, attrs: &BTreeMap<String, String>, exclude: &[String]) -> Result<Loaded, String> {
    let src = attr(attrs, "src", ".");
    let depth: usize = attr(attrs, "depth", "99").parse().unwrap_or(99);
    let src = src.trim_start_matches("./").trim_end_matches('/').to_string();
    let label = if src.is_empty() { ".".to_string() } else { src.clone() };
    let base = if src.is_empty() { root.to_path_buf() } else { root.join(&src) };
    if !base.is_dir() {
        return Err(format!("no such directory: {}", label));
    }
    let mut files = Vec::new();
    walk_files(&base, "", exclude, &mut files);
    let mut all = BTreeSet::new();
    for f in &files {
        let mut acc = String::new();
        for (k, part) in f.split('/').enumerate() {
            acc = if k == 0 { part.to_string() } else { format!("{}/{}", acc, part) };
            all.insert(acc.clone());
        }
    }
    let visible: Vec<String> = all
        .iter()
        .filter(|p| p.split('/').count() <= depth)
        .cloned()
        .collect();
    let mut drawn = vec![label];
    let mut node = Node::default();
    for p in &visible {
        insert(&mut node, p);
    }
    draw(&node, "", &mut drawn);
    Ok(Loaded {
        snapshot: Some(visible.join("\n")),
        kind: Kind::Text { text: drawn.join("\n"), lang: String::new() },
    })
}

fn walk_files(dir: &Path, rel: &str, exclude: &[String], out: &mut Vec<String>) {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" || name.ends_with(".tmp") {
            continue;
        }
        let r = if rel.is_empty() { name.clone() } else { format!("{}/{}", rel, name) };
        if exclude.contains(&r) {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            walk_files(&p, &r, exclude, out);
        } else {
            out.push(r);
        }
    }
}

#[derive(Default)]
struct Node {
    children: BTreeMap<String, Node>,
}

fn insert(node: &mut Node, path: &str) {
    let mut cur = node;
    for part in path.split('/') {
        cur = cur.children.entry(part.to_string()).or_default();
    }
}

fn draw(node: &Node, prefix: &str, out: &mut Vec<String>) {
    let n = node.children.len();
    for (i, (k, child)) in node.children.iter().enumerate() {
        let last = i == n - 1;
        out.push(format!("{}{} {}", prefix, if last { "└──" } else { "├──" }, k));
        let child_prefix = format!("{}{}", prefix, if last { "    " } else { "│   " });
        draw(child, &child_prefix, out);
    }
}

pub fn csv(root: &Path, attrs: &BTreeMap<String, String>) -> Result<Loaded, String> {
    let src = attr(attrs, "src", "");
    if src.is_empty() {
        return Err("csv loader needs src=<file>".to_string());
    }
    let bytes = fs::read(root.join(&src)).map_err(|_| format!("no such file: {}", src))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let mut lines = text.lines();
    let header: Vec<String> = lines.next().unwrap_or("").split(',').map(str::to_string).collect();
    let rows: Vec<Vec<String>> = lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(',').map(str::to_string).collect())
        .collect();
    Ok(Loaded { snapshot: Some(text), kind: Kind::Rows { header, rows } })
}

pub fn sh(root: &Path, attrs: &BTreeMap<String, String>, trusted: bool) -> Result<Loaded, String> {
    let cmd = attr(attrs, "cmd", "");
    if cmd.is_empty() {
        return Err("sh loader needs cmd=<command>".to_string());
    }
    if !trusted {
        return Err(format!("sh loader is disabled for this repo; enable trust to run `{}`", cmd));
    }
    let out = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(root)
        .output()
        .map_err(|e| format!("sh failed: {}", e))?;
    if !out.status.success() {
        let code = out.status.code().unwrap_or(-1);
        return Err(format!("sh exited {}: {}", code, String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(Loaded {
        snapshot: None,
        kind: Kind::Text {
            text: String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
            lang: attr(attrs, "lang", ""),
        },
    })
}
