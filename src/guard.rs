//! `guard`: whether an edit to a template changes what the tool owns, asked
//! before the edit lands, and `check` of one file, asked after it. The
//! Claude Code hooks call it with the hook's JSON on stdin; an editor or
//! another agent can call it with the proposed text in a file.
//!
//! The tool owns a region's body and closer. An edit may change the prose,
//! add a region, remove one whole, or change an opener, which only makes the
//! region stale. An edit that changes a body or a closer is a hand edit, the
//! thing `run` refuses ([ADR 0005]), caught here before it is written.
//!
//! [ADR 0005]: ../docs/adr/0005-refuse-hand-edited-regions.md

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use similar::{DiffOp, TextDiff};

use crate::loader::Production;
use crate::marker::{self, File, ParseError, Region, Segment, Syntax};
use crate::render::{self, Mode, RegionReport, Rendered};
use crate::report;

/// A region whose body or closer an edit changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Touched {
    /// The opener's 1-based line in the current file.
    pub line: usize,
    /// The opener's column, for a region inside a line.
    pub column: Option<usize>,
    pub name: Option<String>,
    pub loader: String,
    /// The opener without its comment delimiters: `tree src=. depth=2`.
    pub source: String,
    pub body: bool,
    pub closer: bool,
}

impl Touched {
    /// `body`, `closer` or `body+closer`.
    pub fn changed(&self) -> &'static str {
        match (self.body, self.closer) {
            (true, true) => "body+closer",
            (false, true) => "closer",
            _ => "body",
        }
    }

    fn label(&self) -> String {
        match &self.name {
            Some(n) => format!("`{n}`"),
            None => format!(
                "`{}@{}`",
                self.loader,
                marker::place(self.line, self.column)
            ),
        }
    }
}

/// What an edit comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Prose, openers, whole regions added or removed: nothing the tool owns.
    Allowed,
    /// The edit changes these regions' bodies or closers.
    Touches(Vec<Touched>),
    /// The proposed text does not parse, though the current text does.
    Breaks(ParseError),
}

fn regions(file: &File) -> Vec<&Region> {
    file.segments
        .iter()
        .filter_map(|s| match s {
            Segment::Region(r) => Some(r),
            Segment::Prose(_) => None,
        })
        .collect()
}

/// The opener's tokens, as the canonical form spells them.
pub fn source(region: &Region) -> String {
    let c = region.opener.canonical();
    c.trim_start_matches("<!-- computed ")
        .trim_end_matches(" -->")
        .to_string()
}

/// Judges `proposed` against `current`. A current text that does not parse
/// guards nothing: `run` already refuses it and says why.
///
/// The line diff between the two says which of a region's lines changed and
/// where lines were inserted. A region whose body and closer lines are
/// untouched is allowed, whatever happened to its opener. A region whose
/// body or closer lines changed is still allowed when its body and sums
/// survive verbatim elsewhere in the proposed text (it moved), or when its
/// opener changed too and its sums survive nowhere (it was removed whole or
/// rewritten into a new region, which `run` renders over).
pub fn judge(current: &str, proposed: &str, syntax: Syntax) -> Verdict {
    let Ok(before) = marker::parse_as(current, syntax) else {
        return Verdict::Allowed;
    };
    let before = regions(&before);
    if before.is_empty() {
        return Verdict::Allowed;
    }
    let after = match marker::parse_as(proposed, syntax) {
        Ok(f) => f,
        Err(e) => return Verdict::Breaks(e),
    };
    let after = regions(&after);

    // Per current line: changed or deleted. Per gap before a current line:
    // lines inserted there.
    let lines = current.split_inclusive('\n').count();
    let mut changed = vec![false; lines];
    let mut inserted = vec![false; lines + 1];
    let diff = TextDiff::from_lines(current, proposed);
    for op in diff.ops() {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete {
                old_index, old_len, ..
            }
            | DiffOp::Replace {
                old_index, old_len, ..
            } => changed[old_index..old_index + old_len].fill(true),
            DiffOp::Insert { old_index, .. } => inserted[old_index] = true,
        }
    }

    // Regions whose owned lines are untouched claim their proposed twin
    // first, so a duplicate cannot vouch for an edited copy of itself.
    let mut claimed = vec![false; after.len()];
    let twin = |r: &Region, claimed: &[bool]| {
        after.iter().enumerate().position(|(i, a)| {
            !claimed[i] && r.sums.is_some() && a.sums == r.sums && a.body == r.body
        })
    };
    let spans: Vec<(usize, usize)> = before
        .iter()
        .map(|r| (r.line - 1, r.last_line() - 1))
        .collect();
    let owned_touched = |&(opener, closer): &(usize, usize)| {
        // A region inside a line shares it with prose: any change to the
        // line may be its body's, which the twin and sums tests then settle.
        if opener == closer {
            return (changed[opener], false);
        }
        let body = changed[opener + 1..closer].iter().any(|&c| c)
            || inserted[opener + 1..=closer].iter().any(|&i| i);
        let closer = changed.get(closer).copied().unwrap_or(false);
        (body, closer)
    };
    for (r, span) in before.iter().zip(&spans) {
        if owned_touched(span) == (false, false)
            && let Some(i) = twin(r, &claimed)
        {
            claimed[i] = true;
        }
    }

    let mut touched = Vec::new();
    for (r, span) in before.iter().zip(&spans) {
        let (body, closer) = owned_touched(span);
        if !body && !closer {
            continue;
        }
        if let Some(i) = twin(r, &claimed) {
            claimed[i] = true;
            continue;
        }
        let survives = r.sums.is_some() && after.iter().any(|a| a.sums == r.sums);
        if changed[span.0] && !survives {
            continue;
        }
        touched.push(Touched {
            line: r.line,
            column: r.column,
            name: r.opener.name.clone(),
            loader: r.opener.loader.clone(),
            source: source(r),
            body,
            closer,
        });
    }
    if touched.is_empty() {
        Verdict::Allowed
    } else {
        Verdict::Touches(touched)
    }
}

/// The message that denies an edit, one line per region, for the agent.
pub fn refusal(path: &Path, verdict: &Verdict) -> Option<String> {
    let run = format!("computed run {}", path.display());
    match verdict {
        Verdict::Allowed => None,
        Verdict::Breaks(e) => Some(format!(
            "{}:{}: this edit breaks the computed markers: {}. Keep every opener paired with its closer; to remove a region, remove its opener, body and closer together.",
            path.display(),
            e.line,
            e.message
        )),
        Verdict::Touches(touched) => {
            let mut out = String::new();
            for t in touched {
                writeln!(
                    out,
                    "{}:{} {} is owned by computed ({}); this edit changes its {}.",
                    path.display(),
                    marker::place(t.line, t.column),
                    t.label(),
                    t.source,
                    t.changed().replace('+', " and ")
                )
                .unwrap();
            }
            write!(
                out,
                "Edit the source it is computed from, or the opener, then run `{run}`. Prose outside the markers is yours to edit."
            )
            .unwrap();
            Some(out)
        }
    }
}

/// `check` of one template's text, inputs read from disk: what the hooks
/// and the editor report after an edit. `Err` is a file-level error.
pub fn check_text(path: &Path, text: &str) -> Result<Vec<RegionReport>, (usize, String)> {
    let file = crate::survey::target(path).unwrap_or_else(|_| path.to_path_buf());
    let path = file.as_path();
    let mut parsed =
        marker::parse_as(text, Syntax::for_path(path)).map_err(|e| (e.line, e.message))?;
    let mut loaders = Production::for_file(path, &mut parsed);
    match render::file(&parsed, Mode::Check, false, &mut loaders) {
        Rendered::Error { line, message } => Err((line, message)),
        rendered => Ok(rendered.regions().to_vec()),
    }
}

/// Which hook event `--hook` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Hook {
    Pre,
    Post,
}

/// The edit a hook describes: the file, and the text it would hold after.
#[derive(Debug, PartialEq, Eq)]
struct Edit {
    file: PathBuf,
    /// `None` when the edit cannot apply (the old text is missing): the tool
    /// call fails on its own, so there is nothing to guard.
    proposed: Option<String>,
}

/// Reads the file path and, for `pre`, the proposed text from a Claude Code
/// `Edit`, `MultiEdit` or `Write` hook input. `None`: not a file edit.
fn edit(input: &Value, current: Option<&str>) -> Option<Edit> {
    let tool = input.get("tool_input")?;
    let path = PathBuf::from(tool.get("file_path")?.as_str()?);
    let file = match input.get("cwd").and_then(Value::as_str) {
        Some(cwd) if path.is_relative() => Path::new(cwd).join(path),
        _ => path,
    };
    let proposed = match input.get("tool_name").and_then(Value::as_str)? {
        "Write" => tool
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string),
        "Edit" => current.and_then(|c| apply(c, tool)),
        "MultiEdit" => current.and_then(|c| {
            tool.get("edits")?
                .as_array()?
                .iter()
                .try_fold(c.to_string(), |text, e| apply(&text, e))
        }),
        _ => return None,
    };
    Some(Edit { file, proposed })
}

/// One `old_string` → `new_string` replacement, as the Edit tool makes it.
fn apply(text: &str, edit: &Value) -> Option<String> {
    let old = edit.get("old_string")?.as_str()?;
    let new = edit.get("new_string")?.as_str()?;
    if old.is_empty() || !text.contains(old) {
        return None;
    }
    let all = edit
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(if all {
        text.replace(old, new)
    } else {
        text.replacen(old, new, 1)
    })
}

/// The path as the agent should read it: relative to the session's
/// directory when under it.
fn shown(file: &Path, input: &Value) -> PathBuf {
    input
        .get("cwd")
        .and_then(Value::as_str)
        .and_then(|cwd| file.strip_prefix(cwd).ok())
        .map_or_else(|| file.to_path_buf(), Path::to_path_buf)
}

/// A template's current text: `None` for a missing file, one that is not
/// UTF-8, or one with no marker, which are none of the guard's business.
fn template(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    marker::has_marker(&text, marker::Syntax::for_path(file)).then_some(text)
}

/// Answers one Claude Code hook: the hook's JSON in, the JSON to print out
/// (`None` prints nothing, which allows the call and adds nothing).
pub fn hook(event: Hook, input: &str) -> Option<String> {
    let input: Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => {
            let message = format!("computed guard: the hook input is not JSON ({e}); not guarded");
            return Some(json!({ "systemMessage": message }).to_string());
        }
    };
    let file = input
        .get("tool_input")
        .and_then(|t| t.get("file_path"))
        .and_then(Value::as_str)?;
    let current = template(&match input.get("cwd").and_then(Value::as_str) {
        Some(cwd) => Path::new(cwd).join(file),
        None => PathBuf::from(file),
    });
    match event {
        Hook::Pre => {
            let current = current?;
            let edit = edit(&input, Some(&current))?;
            let reason = refusal(
                &shown(&edit.file, &input),
                &judge(&current, &edit.proposed?, Syntax::for_path(&edit.file)),
            )?;
            Some(
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": reason,
                    }
                })
                .to_string(),
            )
        }
        Hook::Post => {
            let text = current?;
            let edit = edit(&input, None)?;
            let context = drift(&shown(&edit.file, &input), &edit.file, &text)?;
            Some(
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PostToolUse",
                        "additionalContext": context,
                    }
                })
                .to_string(),
            )
        }
    }
}

/// The context the post-edit hook adds: the `check` lines of a file that is
/// not fresh after the edit, and what to do about them.
fn drift(shown: &Path, file: &Path, text: &str) -> Option<String> {
    match check_text(file, text) {
        Err((line, message)) => Some(format!(
            "{}The computed markers no longer parse; `computed run` refuses this file until they do.",
            report::error(shown, Some(line), &message)
        )),
        Ok(regions) => {
            let lines = report::regions(shown, &regions, Mode::Check, false).concat();
            if lines.is_empty() {
                return None;
            }
            let exec = regions
                .iter()
                .any(|r| render::needs_trust(&r.loader) && r.state.drifted());
            let mut out = format!(
                "computed: regions in {} are not fresh:\n{lines}Run `computed run {}` to render them",
                shown.display(),
                shown.display()
            );
            out.push_str(if exec {
                " (exec and transcript regions run only in a trusted clone: `computed trust`, or `--trust` once)."
            } else {
                "."
            });
            Some(out)
        }
    }
}

/// `computed guard`: with `hook`, answers the Claude Code hook whose JSON is
/// on stdin and exits 0 whatever it decided, since the decision is in the
/// JSON. Otherwise judges `proposed` against `file`: one line per region the
/// edit changes, exit 1 when there is one or the markers break, else 0.
pub fn command(
    file: Option<&Path>,
    proposed: Option<&Path>,
    hook: Option<Hook>,
    json: bool,
) -> std::io::Result<u8> {
    if let Some(event) = hook {
        let mut input = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
        if let Some(out) = self::hook(event, &input) {
            println!("{out}");
        }
        return Ok(0);
    }
    let (Some(file), Some(proposed)) = (file, proposed) else {
        return Err(std::io::Error::other(
            "guard takes FILE and --proposed, or --hook",
        ));
    };
    let read = |p: &Path| {
        std::fs::read_to_string(p)
            .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", p.display())))
    };
    let verdict = judge(&read(file)?, &read(proposed)?, Syntax::for_path(file));
    let exit = u8::from(verdict != Verdict::Allowed);
    if json {
        let (error, regions) = match &verdict {
            Verdict::Allowed => (Value::Null, vec![]),
            Verdict::Breaks(e) => (json!({"line": e.line, "message": e.message}), vec![]),
            Verdict::Touches(t) => (
                Value::Null,
                t.iter()
                    .map(|t| {
                        json!({"line": t.line, "column": t.column, "name": t.name, "loader": t.loader, "changed": t.changed()})
                    })
                    .collect(),
            ),
        };
        let doc = json!({"exit": exit, "path": file.display().to_string(), "error": error, "regions": regions});
        println!("{doc}");
        return Ok(exit);
    }
    match &verdict {
        Verdict::Allowed => {}
        Verdict::Breaks(e) => eprint!("{}", report::error(file, Some(e.line), &e.message)),
        Verdict::Touches(touched) => {
            for t in touched {
                eprintln!(
                    "{}:{} {} {} {} changed",
                    file.display(),
                    marker::place(t.line, t.column),
                    t.name.as_deref().unwrap_or(""),
                    t.loader,
                    t.changed()
                );
            }
        }
    }
    Ok(exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judge(current: &str, proposed: &str) -> Verdict {
        super::judge(current, proposed, Syntax::Markdown)
    }

    const RENDERED: &str = "# Notes\n\n<!-- computed tree src=. name=layout | do not edit; run computed -->\n```text\n.\n└── a\n```\n<!-- /computed in=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa out=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb -->\n\nTail.\n";

    fn touched(v: &Verdict) -> Vec<(usize, &'static str)> {
        match v {
            Verdict::Touches(t) => t.iter().map(|t| (t.line, t.changed())).collect(),
            other => panic!("expected Touches, got {other:?}"),
        }
    }

    #[test]
    fn prose_edits_are_allowed() {
        let p = RENDERED.replace("# Notes", "# Better notes");
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
        let p = RENDERED.replace("Tail.", "Tail.\n\nMore.");
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
        let p = RENDERED.replace("\n\nTail.", "\nTail.");
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
    }

    #[test]
    fn a_body_edit_is_refused() {
        let p = RENDERED.replace("└── a", "└── b");
        assert_eq!(touched(&judge(RENDERED, &p)), [(3, "body")]);
    }

    #[test]
    fn a_line_added_to_the_body_is_refused() {
        let p = RENDERED.replace("```\n<!-- /computed", "```\nextra\n<!-- /computed");
        assert_eq!(touched(&judge(RENDERED, &p)), [(3, "body")]);
        let p = RENDERED.replace("computed -->\n```text", "computed -->\nextra\n```text");
        assert_eq!(touched(&judge(RENDERED, &p)), [(3, "body")]);
    }

    #[test]
    fn lines_added_around_the_region_are_prose() {
        let p = RENDERED.replace("<!-- computed tree", "Before.\n<!-- computed tree");
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
        let p = RENDERED.replace("bbbb -->\n", "bbbb -->\nAfter.\n");
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
    }

    #[test]
    fn a_closer_edit_is_refused() {
        let p = RENDERED.replace("in=aaaa", "in=cccc");
        assert_eq!(touched(&judge(RENDERED, &p)), [(3, "closer")]);
        // Stripping the sums would make the region unrendered by hand.
        let start = RENDERED.find("<!-- /computed in").unwrap();
        let end = start + RENDERED[start..].find('\n').unwrap();
        let p = format!(
            "{}<!-- /computed -->{}",
            &RENDERED[..start],
            &RENDERED[end..]
        );
        assert_eq!(touched(&judge(RENDERED, &p)), [(3, "closer")]);
    }

    #[test]
    fn an_opener_edit_is_allowed() {
        let p = RENDERED.replace("src=. name=layout", "src=. depth=1 name=layout");
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
    }

    #[test]
    fn opener_and_body_together_are_refused() {
        let p = RENDERED
            .replace("src=. name", "src=src name")
            .replace("└── a", "└── b");
        assert_eq!(touched(&judge(RENDERED, &p)), [(3, "body")]);
    }

    #[test]
    fn removing_a_region_whole_is_allowed() {
        let start = RENDERED.find("<!-- computed").unwrap();
        let end = RENDERED.find("bbbb -->\n").unwrap() + "bbbb -->\n".len();
        let p = format!("{}{}", &RENDERED[..start], &RENDERED[end..]);
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
    }

    #[test]
    fn moving_a_region_intact_is_allowed() {
        let start = RENDERED.find("<!-- computed").unwrap();
        let end = RENDERED.find("bbbb -->\n").unwrap() + "bbbb -->\n".len();
        let region = &RENDERED[start..end];
        let p = format!(
            "{}{}\n{region}",
            &RENDERED[..start],
            RENDERED[end..].trim_end()
        );
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
    }

    #[test]
    fn a_duplicate_cannot_vouch_for_an_edited_copy() {
        let start = RENDERED.find("<!-- computed").unwrap();
        let end = RENDERED.find("bbbb -->\n").unwrap() + "bbbb -->\n".len();
        // Names are unique per file, so the copies go unnamed.
        let region = RENDERED[start..end].replace(" name=layout", "");
        let current = format!("{region}\n{region}");
        let p = format!("{region}\n{}", region.replace("└── a", "└── z"));
        assert!(matches!(judge(&current, &p), Verdict::Touches(_)));
    }

    #[test]
    fn a_new_region_is_allowed() {
        let p = format!("{RENDERED}\n<!-- computed tree src=. -->\n<!-- /computed -->\n");
        assert_eq!(judge(RENDERED, &p), Verdict::Allowed);
    }

    #[test]
    fn a_broken_marker_is_named() {
        let p = RENDERED.replace("<!-- /computed in", "<!-- nope in");
        match judge(RENDERED, &p) {
            Verdict::Breaks(e) => assert_eq!(e.line, 3),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_file_without_regions_guards_nothing() {
        assert_eq!(judge("plain\n", "<!-- /computed -->\n"), Verdict::Allowed);
        assert_eq!(
            judge("<!-- computed -->\n", "anything\n"),
            Verdict::Allowed,
            "a file that does not parse now is run's to report"
        );
    }

    #[test]
    fn an_unrendered_body_is_guarded_too() {
        let current = "<!-- computed tree -->\n<!-- /computed -->\n";
        let p = "<!-- computed tree -->\nhand\n<!-- /computed -->\n";
        assert_eq!(touched(&judge(current, p)), [(1, "body")]);
    }

    #[test]
    fn the_refusal_names_the_region_and_its_source() {
        let p = RENDERED.replace("└── a", "└── b");
        let message = refusal(Path::new("CLAUDE.md"), &judge(RENDERED, &p)).unwrap();
        assert!(
            message.starts_with(
                "CLAUDE.md:3 `layout` is owned by computed (tree src=. name=layout); this edit changes its body.\n"
            ),
            "{message}"
        );
        assert!(message.contains("`computed run CLAUDE.md`"), "{message}");
    }

    #[test]
    fn edits_apply_as_the_tools_apply_them() {
        let input = json!({
            "tool_name": "Edit",
            "cwd": "/repo",
            "tool_input": {"file_path": "CLAUDE.md", "old_string": "a", "new_string": "b"}
        });
        let e = edit(&input, Some("a a")).unwrap();
        assert_eq!(e.file, Path::new("/repo/CLAUDE.md"));
        assert_eq!(e.proposed.as_deref(), Some("b a"));

        let input = json!({
            "tool_name": "MultiEdit",
            "tool_input": {"file_path": "/x.md", "edits": [
                {"old_string": "a", "new_string": "b", "replace_all": true},
                {"old_string": "bb", "new_string": "c"}
            ]}
        });
        assert_eq!(
            edit(&input, Some("aa")).unwrap().proposed.as_deref(),
            Some("c")
        );

        let input = json!({
            "tool_name": "Write",
            "tool_input": {"file_path": "/x.md", "content": "new"}
        });
        assert_eq!(
            edit(&input, Some("old")).unwrap().proposed.as_deref(),
            Some("new")
        );

        let input = json!({
            "tool_name": "Edit",
            "tool_input": {"file_path": "/x.md", "old_string": "zz", "new_string": "b"}
        });
        assert_eq!(edit(&input, Some("aa")).unwrap().proposed, None);

        let input = json!({"tool_name": "Read", "tool_input": {"file_path": "/x.md"}});
        assert_eq!(edit(&input, Some("aa")), None);
    }

    #[test]
    fn input_that_is_not_json_is_said_and_allowed() {
        let out = hook(Hook::Pre, "not json").unwrap();
        assert!(
            out.contains("systemMessage") && !out.contains("deny"),
            "{out}"
        );
    }
}
