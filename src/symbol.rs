//! The `symbol` loader: one item of a source file, found by name with
//! tree-sitter, as its whole source, its signature or its doc comment.
//!
//! The grammar is chosen by the extension of `src=`: `.rs` is Rust, `.py`
//! Python, `.go` Go. The snapshot is the extracted text alone, so an edit
//! elsewhere in the file leaves the region fresh.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser};

use crate::loader::{self, Ctx, LoadError, Loaded};
use crate::marker::Opener;

/// Which part of the item the region shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The item's source with the doc comments and attributes above it.
    Whole,
    /// The declaration without docs, attributes or body.
    Signature,
    /// The doc comment's text without its comment markers.
    Doc,
}

impl Part {
    fn parse(s: &str) -> Option<Part> {
        match s {
            "whole" => Some(Part::Whole),
            "signature" => Some(Part::Signature),
            "doc" => Some(Part::Doc),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Part::Whole => "whole",
            Part::Signature => "signature",
            Part::Doc => "doc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolArgs {
    pub src: PathBuf,
    pub item: String,
    pub part: Part,
}

impl SymbolArgs {
    pub fn from_opener(opener: &Opener) -> Result<SymbolArgs, LoadError> {
        validate(opener).map_err(LoadError::Hard)?;
        Ok(SymbolArgs {
            src: PathBuf::from(opener.attr("src").expect("validated")),
            item: opener.attr("item").expect("validated").to_string(),
            part: opener
                .attr("part")
                .map_or(Some(Part::Whole), Part::parse)
                .expect("validated"),
        })
    }
}

/// The opener rules the grammar checks: `src=` and `item=` required, `part=`
/// one of the three.
pub fn validate(opener: &Opener) -> Result<(), String> {
    if opener.attr("src").is_none() {
        return Err("symbol needs src=".to_string());
    }
    match opener.attr("item") {
        None => return Err("symbol needs item=".to_string()),
        Some(item) => {
            ItemPath::parse(item)?;
        }
    }
    match opener.attr("part") {
        Some(p) if Part::parse(p).is_none() => {
            Err(format!("part={p}: expected whole, signature or doc"))
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Python,
    Go,
}

impl Lang {
    fn for_path(path: &Path) -> Option<Lang> {
        match path.extension()?.to_str()? {
            "rs" => Some(Lang::Rust),
            "py" => Some(Lang::Python),
            "go" => Some(Lang::Go),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::Go => "go",
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }
}

/// The fence language `src=` implies, empty for an extension the loader
/// does not read.
pub fn fence_lang(src: &str) -> &'static str {
    Lang::for_path(Path::new(src)).map_or("", Lang::name)
}

/// The item's text and the one-entry snapshot of it: `path#item=…,part=…`,
/// the length and the extracted text. Every error is hard: the tool cannot
/// say what the item is.
pub fn load(
    ctx: &Ctx,
    args: &SymbolArgs,
    read: &mut BTreeSet<PathBuf>,
) -> Result<Loaded, LoadError> {
    let hard = |m: String| LoadError::Hard(m);
    let lang = Lang::for_path(&args.src).ok_or_else(|| {
        hard(format!(
            "src=: {}: no grammar for this extension; symbol reads .rs, .py and .go",
            args.src.display()
        ))
    })?;
    let path = ctx.resolve("src=", &args.src)?;
    if !path.is_file() {
        return Err(hard(format!("src=: {} is not a file", args.src.display())));
    }
    let bytes =
        std::fs::read(&path).map_err(|e| hard(format!("src=: {}: {e}", args.src.display())))?;
    read.insert(path);
    let source = String::from_utf8(bytes)
        .map_err(|_| hard(format!("src=: {} is not UTF-8", args.src.display())))?;
    let item = ItemPath::parse(&args.item).map_err(hard)?;
    let text = extract(lang, &source, &item, args.part)
        .map_err(|m| hard(format!("item={}: {m} in {}", args.item, args.src.display())))?;
    let key = format!(
        "{}#item={},part={}",
        loader::lexical(&args.src).display(),
        args.item,
        args.part.key()
    );
    let mut snapshot = Vec::new();
    loader::push_entry(&mut snapshot, key.as_bytes(), text.as_bytes());
    Ok(Loaded { text, snapshot })
}

/// `item=` parsed: `a::b::name`, `Type.method`, `Trait for Type.method`,
/// with a leading module path on the last two as well.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ItemPath {
    modules: Vec<String>,
    owner: Option<Owner>,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Owner {
    ty: String,
    /// The trait of `Trait for Type`, which picks one impl among several.
    tr: Option<String>,
}

impl ItemPath {
    fn parse(item: &str) -> Result<ItemPath, String> {
        let bad = || {
            format!(
                "item={item}: expected name, module::name, Type.method or \"Trait for Type.method\""
            )
        };
        let (path, member) = match item.rsplit_once('.') {
            Some((path, member)) => (path, Some(member)),
            None => (item, None),
        };
        let mut segments: Vec<&str> = path.split("::").collect();
        let last = segments.pop().ok_or_else(bad)?;
        let modules: Vec<String> = segments.iter().map(|s| s.to_string()).collect();
        let word = |s: &str| !s.is_empty() && !s.contains(char::is_whitespace);
        if !modules.iter().all(|m| word(m)) {
            return Err(bad());
        }
        let (owner, name) = match member {
            None => (None, last),
            Some(member) => {
                let owner = match last.split_once(" for ") {
                    Some((tr, ty)) => Owner {
                        ty: ty.trim().to_string(),
                        tr: Some(tr.trim().to_string()),
                    },
                    None => Owner {
                        ty: last.to_string(),
                        tr: None,
                    },
                };
                if !word(&owner.ty) || owner.tr.as_deref().is_some_and(|t| !word(t)) {
                    return Err(bad());
                }
                (Some(owner), member)
            }
        };
        if !word(name) {
            return Err(bad());
        }
        Ok(ItemPath {
            modules,
            owner,
            name: name.to_string(),
        })
    }
}

/// One definition found in a container.
#[derive(Clone, Copy)]
struct Def<'t> {
    /// The definition itself, where `signature` starts.
    node: Node<'t>,
    /// The node that holds it with its decorators, where `whole` starts
    /// when no doc comment or attribute sits above it.
    outer: Node<'t>,
}

/// The text of `part` of the item `path` names in `source`.
fn extract(lang: Lang, source: &str, path: &ItemPath, part: Part) -> Result<String, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&lang.grammar())
        .map_err(|e| format!("{} grammar: {e}", lang.name()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "the parser gave up".to_string())?;
    let root = tree.root_node();
    let def = find(lang, root, source, path).map_err(|m| {
        if root.has_error() {
            format!("{m}; the file does not parse cleanly")
        } else {
            m
        }
    })?;
    match part {
        Part::Whole => {
            let start = attached(lang, def)
                .first()
                .map_or(def.outer.start_byte(), Node::start_byte);
            Ok(dedent(source, start, def.outer.end_byte()))
        }
        Part::Signature => {
            let end = signature_end(lang, source, def.node);
            Ok(dedent(source, def.node.start_byte(), end))
        }
        Part::Doc => doc(lang, source, def).ok_or_else(|| "has no doc comment".to_string()),
    }
}

fn text<'s>(node: Node<'_>, source: &'s str) -> &'s str {
    &source[node.byte_range()]
}

fn named_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}

fn find<'t>(lang: Lang, root: Node<'t>, source: &str, path: &ItemPath) -> Result<Def<'t>, String> {
    let mut container = root;
    for module in &path.modules {
        if lang != Lang::Rust {
            return Err(format!(
                "`::` names a Rust module; {} has none",
                lang.name()
            ));
        }
        let bodies: Vec<Node<'t>> = named_children(container)
            .into_iter()
            .filter(|n| n.kind() == "mod_item" && name_of(*n, source) == Some(module.as_str()))
            .filter_map(|n| n.child_by_field_name("body"))
            .collect();
        container = match bodies.as_slice() {
            [] => return Err(format!("no inline module named {module}")),
            [body] => *body,
            _ => return Err(format!("more than one module named {module}")),
        };
    }
    let candidates: Vec<(String, Def<'t>)> = match &path.owner {
        None => defs(lang, container, source)
            .into_iter()
            .filter(|(name, _)| *name == path.name)
            .map(|(_, d)| (format!("line {}", line(d.outer)), d))
            .collect(),
        Some(owner) => methods(lang, container, source, owner)?
            .into_iter()
            .flat_map(|(label, body)| {
                defs(lang, body, source)
                    .into_iter()
                    .filter(|(name, _)| *name == path.name)
                    .map(move |(_, d)| (format!("{label} at line {}", line(d.outer)), d))
            })
            .chain(
                go_methods(lang, container, source, owner)
                    .into_iter()
                    .filter(|(name, _)| *name == path.name)
                    .map(|(_, d)| (format!("line {}", line(d.outer)), d)),
            )
            .collect(),
    };
    match candidates.as_slice() {
        [] => Err("no such item".to_string()),
        [(_, def)] => Ok(*def),
        _ => {
            let list: Vec<&str> = candidates.iter().map(|(l, _)| l.as_str()).collect();
            let hint = if lang == Lang::Rust && path.owner.as_ref().is_some_and(|o| o.tr.is_none())
            {
                "; name one as \"Trait for Type.method\""
            } else {
                ""
            };
            Err(format!("ambiguous: {}{hint}", list.join(", ")))
        }
    }
}

/// The `name` field's text.
fn name_of<'s>(node: Node<'_>, source: &'s str) -> Option<&'s str> {
    node.child_by_field_name("name").map(|n| text(n, source))
}

/// The named definitions directly inside `container`.
fn defs<'t>(lang: Lang, container: Node<'t>, source: &str) -> Vec<(String, Def<'t>)> {
    let mut out = Vec::new();
    for node in named_children(container) {
        let def = Def { node, outer: node };
        match (lang, node.kind()) {
            (
                Lang::Rust,
                "function_item"
                | "function_signature_item"
                | "struct_item"
                | "enum_item"
                | "union_item"
                | "trait_item"
                | "const_item"
                | "static_item"
                | "type_item"
                | "mod_item"
                | "macro_definition"
                | "associated_type",
            )
            | (Lang::Python, "function_definition" | "class_definition")
            | (Lang::Go, "function_declaration") => {
                if let Some(name) = name_of(node, source) {
                    out.push((name.to_string(), def));
                }
            }
            (Lang::Python, "decorated_definition") => {
                if let Some(inner) = node.child_by_field_name("definition")
                    && let Some(name) = name_of(inner, source)
                {
                    out.push((
                        name.to_string(),
                        Def {
                            node: inner,
                            outer: node,
                        },
                    ));
                }
            }
            (Lang::Python, "expression_statement") => {
                if let Some(assign) = node.named_child(0).filter(|a| a.kind() == "assignment")
                    && let Some(left) = assign.child_by_field_name("left")
                    && left.kind() == "identifier"
                {
                    out.push((text(left, source).to_string(), def));
                }
            }
            (Lang::Go, "type_declaration" | "const_declaration" | "var_declaration") => {
                let specs: Vec<Node<'t>> = named_children(node)
                    .into_iter()
                    .filter(|s| s.kind() != "comment")
                    .collect();
                for spec in &specs {
                    let mut cursor = spec.walk();
                    for name in spec.children_by_field_name("name", &mut cursor) {
                        // A declaration of one spec is the item; in a group,
                        // the spec is.
                        let def = if specs.len() == 1 {
                            def
                        } else {
                            Def {
                                node: *spec,
                                outer: *spec,
                            }
                        };
                        out.push((text(name, source).to_string(), def));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The containers whose members are `Type.member`: in Rust every impl of
/// the type, narrowed by trait when one is named, and a trait of that name;
/// in Python the class. Labelled for an ambiguity message.
fn methods<'t>(
    lang: Lang,
    container: Node<'t>,
    source: &str,
    owner: &Owner,
) -> Result<Vec<(String, Node<'t>)>, String> {
    if lang != Lang::Rust && owner.tr.is_some() {
        return Err(format!(
            "\"Trait for Type\" is Rust; {} has no impls",
            lang.name()
        ));
    }
    let mut out = Vec::new();
    for node in named_children(container) {
        // A decorated Python class is the class.
        let node = match node.kind() {
            "decorated_definition" => match node.child_by_field_name("definition") {
                Some(inner) => inner,
                None => continue,
            },
            _ => node,
        };
        let Some(body) = node.child_by_field_name("body") else {
            continue;
        };
        match (lang, node.kind()) {
            (Lang::Rust, "impl_item") => {
                let ty = node
                    .child_by_field_name("type")
                    .map(|t| base_name(text(t, source)));
                let tr = node
                    .child_by_field_name("trait")
                    .map(|t| base_name(text(t, source)));
                let wanted_trait = owner.tr.as_deref().map(base_name);
                if ty == Some(base_name(&owner.ty))
                    && (wanted_trait.is_none() || tr == wanted_trait)
                {
                    let header =
                        text(node, source)[..body.start_byte() - node.start_byte()].trim_end();
                    out.push((header.to_string(), body));
                }
            }
            (Lang::Rust, "trait_item")
                if owner.tr.is_none() && name_of(node, source) == Some(owner.ty.as_str()) =>
            {
                out.push((format!("trait {}", owner.ty), body));
            }
            (Lang::Python, "class_definition")
                if name_of(node, source) == Some(owner.ty.as_str()) =>
            {
                out.push((format!("class {}", owner.ty), body));
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Go's methods sit at the top level with a receiver: `func (t *T) M()`.
fn go_methods<'t>(
    lang: Lang,
    container: Node<'t>,
    source: &str,
    owner: &Owner,
) -> Vec<(String, Def<'t>)> {
    if lang != Lang::Go {
        return Vec::new();
    }
    named_children(container)
        .into_iter()
        .filter(|n| n.kind() == "method_declaration")
        .filter(|n| {
            n.child_by_field_name("receiver")
                .and_then(|r| r.named_child(0))
                .and_then(|p| p.child_by_field_name("type"))
                .is_some_and(|t| base_name(text(t, source).trim_start_matches('*')) == owner.ty)
        })
        .filter_map(|n| Some((name_of(n, source)?.to_string(), Def { node: n, outer: n })))
        .collect()
}

/// A type as the impl or receiver spells it, reduced to its last path
/// segment without generics or references: `&mut a::Foo<T>` is `Foo`.
fn base_name(ty: &str) -> &str {
    let ty = ty.trim_start_matches('&').trim_start();
    let ty = ty.strip_prefix("mut ").unwrap_or(ty);
    let ty = ty.strip_prefix("dyn ").unwrap_or(ty);
    let ty = ty.split(['<', '[']).next().unwrap_or(ty);
    ty.rsplit("::").next().unwrap_or(ty).trim()
}

/// The doc comments and attributes attached above a definition, top first:
/// in Rust every outer doc comment and attribute up to the first other
/// node, in Go the comment lines directly above with no blank line between.
/// Python's decorators are part of the definition's outer node.
fn attached<'t>(lang: Lang, def: Def<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut current = def.outer;
    while let Some(prev) = current.prev_named_sibling() {
        let take = match lang {
            Lang::Rust => match prev.kind() {
                "attribute_item" => true,
                "line_comment" | "block_comment" => prev.child_by_field_name("outer").is_some(),
                _ => false,
            },
            Lang::Go => {
                prev.kind() == "comment"
                    && prev.end_position().row + 1 == current.start_position().row
            }
            Lang::Python => false,
        };
        if !take {
            break;
        }
        out.push(prev);
        current = prev;
    }
    out.reverse();
    out
}

/// Where a signature ends: before a body in braces or an indented block,
/// before the `=` of a value, else before a closing `;`.
fn signature_end(lang: Lang, source: &str, node: Node<'_>) -> usize {
    let trim = |end: usize, chars: &[char]| {
        source[node.start_byte()..end]
            .trim_end_matches(|c: char| c.is_whitespace() || chars.contains(&c))
            .len()
            + node.start_byte()
    };
    if lang == Lang::Rust && node.kind() == "macro_definition" {
        return node
            .child_by_field_name("name")
            .map_or(node.end_byte(), |n| n.end_byte());
    }
    if let Some(body) = node.child_by_field_name("body") {
        let block = matches!(
            body.kind(),
            "block" | "field_declaration_list" | "enum_variant_list" | "declaration_list"
        );
        if block {
            return trim(body.start_byte(), &[':']);
        }
    }
    if lang == Lang::Go
        && node.kind() == "type_declaration"
        && let Some(ty) = node
            .named_child(0)
            .and_then(|s| s.child_by_field_name("type"))
        && matches!(ty.kind(), "struct_type" | "interface_type")
        && let Some(brace) = text(ty, source).find('{')
    {
        return trim(ty.start_byte() + brace, &[]);
    }
    let value = match (lang, node.kind()) {
        (Lang::Go, "const_declaration" | "var_declaration") => node
            .named_child(0)
            .and_then(|s| s.child_by_field_name("value")),
        (Lang::Python, "expression_statement") => node
            .named_child(0)
            .and_then(|a| a.child_by_field_name("right")),
        _ => node.child_by_field_name("value"),
    };
    match value {
        Some(v) => trim(v.start_byte(), &['=']),
        None => trim(node.end_byte(), &[';']),
    }
}

/// The source from `start` to `end`, from the start of `start`'s line when
/// only indentation precedes it, with that indentation taken off every line
/// that has it.
fn dedent(source: &str, start: usize, end: usize) -> String {
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let lead = &source[line_start..start];
    let indent = if lead.chars().all(|c| c == ' ' || c == '\t') {
        lead
    } else {
        let rest = &source[line_start..];
        &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()]
    };
    let body = source[start..end].trim_end();
    let mut out = String::with_capacity(body.len());
    for (i, l) in body.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
            out.push_str(l.strip_prefix(indent).unwrap_or(l));
        } else {
            out.push_str(l);
        }
    }
    out
}

/// The doc comment's text: Rust's `///` and `/** */`, Python's docstring,
/// Go's comment lines above, markers taken off. `None` when there is none.
fn doc(lang: Lang, source: &str, def: Def<'_>) -> Option<String> {
    let lines: Vec<String> = match lang {
        Lang::Rust => attached(lang, def)
            .into_iter()
            .filter_map(|n| {
                n.child_by_field_name("doc")
                    .map(|d| (n.kind(), text(d, source)))
            })
            .flat_map(|(kind, d)| match kind {
                "block_comment" => block_lines(d),
                _ => vec![strip_one_space(d.trim_end_matches(['\n', '\r'])).to_string()],
            })
            .collect(),
        Lang::Go => attached(lang, def)
            .into_iter()
            .map(|n| text(n, source))
            .filter(|c| !c.starts_with("//go:"))
            .flat_map(|c| match c.strip_prefix("//") {
                Some(l) => vec![strip_one_space(l).to_string()],
                None => block_lines(c.trim_start_matches("/*").trim_end_matches("*/")),
            })
            .collect(),
        Lang::Python => {
            let first = def.node.child_by_field_name("body")?.named_child(0)?;
            let string = first.named_child(0).filter(|s| {
                first.kind() == "expression_statement"
                    && first.named_child_count() == 1
                    && s.kind() == "string"
            })?;
            docstring(text(string, source))?
        }
    };
    let start = lines.iter().position(|l| !l.trim().is_empty())?;
    let end = lines.iter().rposition(|l| !l.trim().is_empty())? + 1;
    Some(lines[start..end].join("\n"))
}

fn strip_one_space(s: &str) -> &str {
    s.strip_prefix(' ').unwrap_or(s)
}

/// The lines of a block comment's inside, a leading ` * ` taken off each
/// continuation line that has one.
fn block_lines(inside: &str) -> Vec<String> {
    inside
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                return strip_one_space(l).trim_end().to_string();
            }
            let t = l.trim_start();
            match t.strip_prefix('*') {
                Some(rest) => strip_one_space(rest).trim_end().to_string(),
                None => l.trim_end().to_string(),
            }
        })
        .collect()
}

/// A Python string literal's content, cleaned as `inspect.cleandoc` does:
/// the first line stripped, the common indentation of the others removed.
/// Escapes are left as written.
fn docstring(literal: &str) -> Option<Vec<String>> {
    let body = literal.trim_start_matches(['r', 'R', 'u', 'U']);
    let quote = ["\"\"\"", "'''", "\"", "'"]
        .into_iter()
        .find(|q| body.starts_with(q) && body.ends_with(q) && body.len() >= 2 * q.len())?;
    let inner = &body[quote.len()..body.len() - quote.len()];
    let mut lines: Vec<&str> = inner.lines().collect();
    if inner.ends_with('\n') {
        lines.push("");
    }
    let indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    Some(
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                if i == 0 {
                    l.trim().to_string()
                } else {
                    l.get(indent..).unwrap_or("").trim_end().to_string()
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: &str = r#"//! A module.

use std::fmt;

/// The widget.
///
/// Holds one value.
#[derive(Debug)]
pub struct Widget<T>
where
    T: Clone,
{
    /// The value.
    pub value: T,
}

/** Block doc,
 * two lines. */
pub const LIMIT: u32 = 5;

impl<T: Clone> Widget<T> {
    /// Makes one.
    #[must_use]
    pub fn new(value: T) -> Self {
        Widget { value }
    }

    pub fn get(&self) -> &T {
        &self.value
    }
}

impl fmt::Display for Widget<u8> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl fmt::Debug for Other {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl fmt::Display for Other {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

pub trait Shape {
    /// The area.
    fn area(&self) -> f64;
}

pub struct Unit;

pub enum Kind {
    A,
    B(u8),
}

mod inner {
    // Not a doc comment.
    pub fn helper(x: u8) -> u8 { x }
}

macro_rules! twice {
    ($e:expr) => { $e * 2 };
}
"#;

    fn get(source: &str, lang: Lang, item: &str, part: Part) -> Result<String, String> {
        extract(lang, source, &ItemPath::parse(item).unwrap(), part)
    }

    fn rust(item: &str, part: Part) -> Result<String, String> {
        get(RUST, Lang::Rust, item, part)
    }

    #[test]
    fn whole_takes_the_doc_comments_and_attributes_above() {
        assert_eq!(
            rust("Widget", Part::Whole).unwrap(),
            "/// The widget.\n///\n/// Holds one value.\n#[derive(Debug)]\npub struct Widget<T>\nwhere\n    T: Clone,\n{\n    /// The value.\n    pub value: T,\n}"
        );
        assert_eq!(
            rust("LIMIT", Part::Whole).unwrap(),
            "/** Block doc,\n * two lines. */\npub const LIMIT: u32 = 5;"
        );
        assert_eq!(rust("Unit", Part::Whole).unwrap(), "pub struct Unit;");
        assert_eq!(
            rust("inner::helper", Part::Whole).unwrap(),
            "pub fn helper(x: u8) -> u8 { x }",
            "a plain comment is not attached"
        );
    }

    #[test]
    fn a_method_is_dedented_out_of_its_impl() {
        assert_eq!(
            rust("Widget.new", Part::Whole).unwrap(),
            "/// Makes one.\n#[must_use]\npub fn new(value: T) -> Self {\n    Widget { value }\n}"
        );
        assert_eq!(
            rust("Widget.fmt", Part::Signature).unwrap(),
            "fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result"
        );
        assert_eq!(
            rust("Shape.area", Part::Whole).unwrap(),
            "/// The area.\nfn area(&self) -> f64;"
        );
    }

    #[test]
    fn signature_stops_before_the_body_or_value() {
        let cases = [
            ("Widget.new", "pub fn new(value: T) -> Self"),
            ("Widget", "pub struct Widget<T>\nwhere\n    T: Clone,"),
            ("LIMIT", "pub const LIMIT: u32"),
            ("Shape", "pub trait Shape"),
            ("Shape.area", "fn area(&self) -> f64"),
            ("Unit", "pub struct Unit"),
            ("Kind", "pub enum Kind"),
            ("inner", "mod inner"),
            ("twice", "macro_rules! twice"),
        ];
        for (item, want) in cases {
            assert_eq!(rust(item, Part::Signature).unwrap(), want, "{item}");
        }
    }

    #[test]
    fn doc_is_the_comment_text_without_markers() {
        assert_eq!(
            rust("Widget", Part::Doc).unwrap(),
            "The widget.\n\nHolds one value."
        );
        assert_eq!(rust("LIMIT", Part::Doc).unwrap(), "Block doc,\ntwo lines.");
        assert_eq!(
            rust("Widget.get", Part::Doc).unwrap_err(),
            "has no doc comment"
        );
    }

    #[test]
    fn an_ambiguous_method_lists_its_candidates_and_a_trait_picks_one() {
        let e = rust("Other.fmt", Part::Whole).unwrap_err();
        assert!(
            e.starts_with("ambiguous: impl fmt::Debug for Other at line 40, impl fmt::Display for Other at line 46"),
            "{e}"
        );
        assert!(e.contains("Trait for Type.method"), "{e}");
        assert!(
            rust("Debug for Other.fmt", Part::Whole)
                .unwrap()
                .starts_with("fn fmt")
        );
        assert_eq!(
            rust("fmt::Display for Other.fmt", Part::Whole).unwrap_err(),
            "no inline module named fmt",
            "a trait is named by its last segment; `::` before it is a module path"
        );
    }

    #[test]
    fn a_missing_item_is_an_error() {
        assert_eq!(rust("Nope", Part::Whole).unwrap_err(), "no such item");
        assert_eq!(
            rust("Widget.nope", Part::Whole).unwrap_err(),
            "no such item"
        );
        assert_eq!(
            rust("nope::helper", Part::Whole).unwrap_err(),
            "no inline module named nope"
        );
        assert!(
            get("fn broken( {\n", Lang::Rust, "x", Part::Whole)
                .unwrap_err()
                .contains("does not parse cleanly")
        );
    }

    #[test]
    fn item_paths_parse() {
        assert_eq!(
            ItemPath::parse("a::b::Foo.bar").unwrap(),
            ItemPath {
                modules: vec!["a".into(), "b".into()],
                owner: Some(Owner {
                    ty: "Foo".into(),
                    tr: None
                }),
                name: "bar".into(),
            }
        );
        assert_eq!(
            ItemPath::parse("Display for Foo.fmt").unwrap().owner,
            Some(Owner {
                ty: "Foo".into(),
                tr: Some("Display".into())
            })
        );
        for bad in ["", "a::", "::a", "Foo.", ".bar", "a b"] {
            assert!(ItemPath::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_sink_and_language_default_from_the_part_and_the_extension() {
        use crate::marker::Sink;
        let opener = |attrs: &str| {
            let text = format!("<!-- computed symbol {attrs} -->\n<!-- /computed -->\n");
            match crate::marker::parse(&text).map(|f| f.segments.into_iter().next()) {
                Ok(Some(crate::marker::Segment::Region(r))) => Ok((r.opener.sink, r.opener.lang)),
                Ok(_) => unreachable!(),
                Err(e) => Err(e.message),
            }
        };
        assert_eq!(opener("src=a.rs item=x"), Ok((Sink::Fence, "rust".into())));
        assert_eq!(
            opener("src=a.py item=x"),
            Ok((Sink::Fence, "python".into()))
        );
        assert_eq!(
            opener("src=a.go item=x part=signature"),
            Ok((Sink::Fence, "go".into()))
        );
        assert_eq!(
            opener("src=a.rs item=x part=doc"),
            Ok((Sink::Raw, "".into()))
        );
        assert_eq!(
            opener("src=a.rs item=x part=doc as=fence lang=markdown"),
            Ok((Sink::Fence, "markdown".into()))
        );
        assert_eq!(
            opener("src=a.rs item=x lang=\"\""),
            Ok((Sink::Fence, "".into()))
        );
        assert_eq!(opener("src=a.txt item=x"), Ok((Sink::Fence, "".into())));
        assert_eq!(opener("item=x"), Err("symbol needs src=".into()));
        assert_eq!(opener("src=a.rs"), Err("symbol needs item=".into()));
        assert_eq!(
            opener("src=a.rs item=x part=body"),
            Err("part=body: expected whole, signature or doc".into())
        );
    }

    const PYTHON: &str = r#"import os

@decorator
def area(width: int,
         height: int) -> int:
    """Width times height.

    Both in metres.
    """
    return width * height


@dataclass
class Shape(Base):
    '''A shape.'''

    def grow(self, by):
        return by


LIMIT = 5
"#;

    #[test]
    fn python_functions_classes_and_docstrings() {
        let py = |item, part| get(PYTHON, Lang::Python, item, part);
        assert_eq!(
            py("area", Part::Whole).unwrap(),
            "@decorator\ndef area(width: int,\n         height: int) -> int:\n    \"\"\"Width times height.\n\n    Both in metres.\n    \"\"\"\n    return width * height"
        );
        assert_eq!(
            py("area", Part::Signature).unwrap(),
            "def area(width: int,\n         height: int) -> int"
        );
        assert_eq!(
            py("area", Part::Doc).unwrap(),
            "Width times height.\n\nBoth in metres."
        );
        assert_eq!(py("Shape", Part::Doc).unwrap(), "A shape.");
        assert_eq!(py("Shape", Part::Signature).unwrap(), "class Shape(Base)");
        assert_eq!(
            py("Shape.grow", Part::Whole).unwrap(),
            "def grow(self, by):\n    return by"
        );
        assert_eq!(py("LIMIT", Part::Signature).unwrap(), "LIMIT");
        assert!(
            py("x::area", Part::Whole)
                .unwrap_err()
                .contains("Rust module")
        );
    }

    const GO: &str = "package shapes\n\n// Area is width times height.\n// In metres.\nfunc Area(w, h int) int {\n\treturn w * h\n}\n\n// Shape is a thing.\n\ntype Shape struct {\n\tW int\n}\n\n// Grow grows it.\nfunc (s *Shape) Grow(by int) {\n\ts.W += by\n}\n\nconst (\n\t// Max is the most.\n\tMax = 10\n\tMin = 0\n)\n";

    #[test]
    fn go_functions_methods_types_and_comments() {
        let go = |item, part| get(GO, Lang::Go, item, part);
        assert_eq!(
            go("Area", Part::Whole).unwrap(),
            "// Area is width times height.\n// In metres.\nfunc Area(w, h int) int {\n\treturn w * h\n}"
        );
        assert_eq!(
            go("Area", Part::Signature).unwrap(),
            "func Area(w, h int) int"
        );
        assert_eq!(
            go("Area", Part::Doc).unwrap(),
            "Area is width times height.\nIn metres."
        );
        assert_eq!(
            go("Shape", Part::Doc).unwrap_err(),
            "has no doc comment",
            "a blank line detaches a comment"
        );
        assert_eq!(go("Shape", Part::Signature).unwrap(), "type Shape struct");
        assert_eq!(
            go("Shape.Grow", Part::Signature).unwrap(),
            "func (s *Shape) Grow(by int)"
        );
        assert_eq!(go("Shape.Grow", Part::Doc).unwrap(), "Grow grows it.");
        assert_eq!(
            go("Max", Part::Whole).unwrap(),
            "// Max is the most.\nMax = 10"
        );
        assert_eq!(go("Min", Part::Signature).unwrap(), "Min");
    }
}
