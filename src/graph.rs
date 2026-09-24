//! `graph`: templates, their regions and what each region reads, as a
//! Mermaid flowchart, a Graphviz digraph or JSON. A region that reads
//! another template in the graph points at that template with a settling
//! edge: `run` passes over it again once the other is written.
//!
//! An input is drawn as the opener names it, so a glob stays one node: a
//! tree's `src=` directory, an `inputs=` glob, a file's `src=`, and, for any
//! other loader, the files its snapshot read.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::affected::{self, Reach};
use crate::loader::Production;
use crate::marker::Region;
use crate::report;
use crate::survey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Mermaid,
    Dot,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Template,
    Region,
    Input,
}

struct Node {
    id: String,
    kind: Kind,
    label: String,
    /// A region's template and opener line.
    at: Option<(String, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    /// A template holds a region.
    Holds,
    /// A region reads an input.
    Reads,
    /// A region reads another template, so `run` settles the two.
    Settles,
}

impl Edge {
    fn key(self) -> &'static str {
        match self {
            Edge::Holds => "holds",
            Edge::Reads => "reads",
            Edge::Settles => "settles",
        }
    }
}

/// Prints the graph of the templates under `paths`. Exit 0, or 2 when a
/// template could not be read; the graph of the others is still printed.
pub fn main(paths: &[PathBuf], style: Style) -> Result<u8, String> {
    let (templates, errors) = survey::templates(paths)?;
    let canonical: Vec<Option<PathBuf>> = templates
        .iter()
        .map(|t| t.file.canonicalize().ok())
        .collect();
    let mut nodes: Vec<Node> = Vec::new();
    for (i, t) in templates.iter().enumerate() {
        nodes.push(Node {
            id: format!("t{i}"),
            kind: Kind::Template,
            label: t.path.display().to_string(),
            at: None,
        });
    }
    let mut holds = Vec::new();
    let mut reads = Vec::new();
    let mut inputs: HashMap<String, String> = HashMap::new();
    let mut input_nodes = Vec::new();
    let mut region = 0;
    for (ti, t) in templates.iter().enumerate() {
        let mut loaders = Production::new(t.ctx.clone());
        for r in t.regions() {
            let id = format!("r{region}");
            region += 1;
            nodes.push(Node {
                id: id.clone(),
                kind: Kind::Region,
                label: region_label(r),
                at: Some((t.path.display().to_string(), r.line)),
            });
            holds.push((format!("t{ti}"), id.clone(), Edge::Holds));
            let reach = affected::reach(t, r, &mut loaders);
            let settles: Vec<usize> = canonical
                .iter()
                .enumerate()
                .filter(|(_, c)| c.as_ref().is_some_and(|c| reach.files.contains(c)))
                .map(|(i, _)| i)
                .collect();
            for (label, path) in named_inputs(&reach) {
                if settles
                    .iter()
                    .any(|&s| path.is_some() && canonical[s] == path)
                {
                    continue;
                }
                let next = format!("i{}", inputs.len());
                let input = inputs.entry(label.clone()).or_insert_with(|| {
                    input_nodes.push(Node {
                        id: next.clone(),
                        kind: Kind::Input,
                        label,
                        at: None,
                    });
                    next
                });
                reads.push((id.clone(), input.clone(), Edge::Reads));
            }
            for s in settles {
                reads.push((id.clone(), format!("t{s}"), Edge::Settles));
            }
        }
    }
    nodes.extend(input_nodes);
    let edges: Vec<(String, String, Edge)> = holds.into_iter().chain(reads).collect();
    let exit = if errors.is_empty() { 0 } else { 2 };
    let text = match style {
        Style::Mermaid => mermaid(&nodes, &edges),
        Style::Dot => dot(&nodes, &edges),
        Style::Json => json(&nodes, &edges, &affected::errors_json(&errors), exit),
    };
    if style != Style::Json {
        affected::print_errors(&errors);
    }
    print!("{text}");
    Ok(exit)
}

/// `name · loader`, `line N · loader` for a region without a name, with
/// `volatile` when the region reads nothing by declaration.
fn region_label(r: &Region) -> String {
    let name = r
        .opener
        .name
        .clone()
        .unwrap_or_else(|| format!("line {}", r.line));
    let volatile = if r.opener.flag("volatile") {
        " volatile"
    } else {
        ""
    };
    format!("{name} · {}{volatile}", r.opener.loader)
}

/// The inputs as the opener names them, each with its label and, when it
/// names one path, that path anchored.
fn named_inputs(reach: &Reach) -> Vec<(String, Option<PathBuf>)> {
    if let Some(l) = &reach.listing {
        let dir = survey::display(&l.dir);
        let label = if dir.ends_with('/') {
            dir
        } else {
            format!("{dir}/")
        };
        return vec![(label, Some(l.dir.clone()))];
    }
    if let Some(src) = &reach.src {
        return vec![(survey::display(src), Some(src.clone()))];
    }
    if !reach.globs.is_empty() {
        return reach
            .globs
            .iter()
            .map(|g| {
                let path = survey::normalise(&reach.root.join(g.trim().trim_end_matches('/')));
                (survey::display(&path), Some(path))
            })
            .collect();
    }
    reach
        .files
        .iter()
        .map(|f| (survey::display(f), Some(f.clone())))
        .collect()
}

fn mermaid(nodes: &[Node], edges: &[(String, String, Edge)]) -> String {
    let mut out = String::from("flowchart LR\n");
    for n in nodes {
        let label = n.label.replace('"', "#quot;");
        let shape = match n.kind {
            Kind::Template => format!("[\"{label}\"]"),
            Kind::Region => format!("([\"{label}\"])"),
            Kind::Input => format!("[/\"{label}\"/]"),
        };
        writeln!(out, "  {}{shape}", n.id).unwrap();
    }
    for (from, to, edge) in edges {
        let arrow = match edge {
            Edge::Holds | Edge::Reads => "-->",
            Edge::Settles => "-.->|settles|",
        };
        writeln!(out, "  {from} {arrow} {to}").unwrap();
    }
    out
}

fn dot(nodes: &[Node], edges: &[(String, String, Edge)]) -> String {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut out = String::from("digraph computed {\n  rankdir=LR;\n");
    for n in nodes {
        let shape = match n.kind {
            Kind::Template => "shape=box",
            Kind::Region => "shape=box, style=rounded",
            Kind::Input => "shape=note",
        };
        writeln!(out, "  {} [label={}, {shape}];", n.id, quote(&n.label)).unwrap();
    }
    for (from, to, edge) in edges {
        match edge {
            Edge::Holds | Edge::Reads => writeln!(out, "  {from} -> {to};").unwrap(),
            Edge::Settles => {
                writeln!(out, "  {from} -> {to} [style=dashed, label=\"settles\"];").unwrap()
            }
        }
    }
    out.push_str("}\n");
    out
}

fn json(nodes: &[Node], edges: &[(String, String, Edge)], errors: &str, exit: u8) -> String {
    let nodes: Vec<String> = nodes
        .iter()
        .map(|n| {
            let kind = match n.kind {
                Kind::Template => "template",
                Kind::Region => "region",
                Kind::Input => "input",
            };
            let mut s = format!(
                "{{\"id\":\"{}\",\"kind\":\"{kind}\",\"label\":{}",
                n.id,
                report::string(&n.label)
            );
            if let Some((path, line)) = &n.at {
                write!(s, ",\"path\":{},\"line\":{line}", report::string(path)).unwrap();
            }
            s.push('}');
            s
        })
        .collect();
    let edges: Vec<String> = edges
        .iter()
        .map(|(from, to, edge)| {
            format!(
                "{{\"from\":\"{from}\",\"to\":\"{to}\",\"kind\":\"{}\"}}",
                edge.key()
            )
        })
        .collect();
    format!(
        "{{\"exit\":{exit},\"errors\":{errors},\"nodes\":[{}],\"edges\":[{}]}}\n",
        nodes.join(","),
        edges.join(",")
    )
}
