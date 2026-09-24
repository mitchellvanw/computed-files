//! `lsp`: a language server over stdio, so an editor shows every region's
//! state as the file is edited.
//!
//! Diagnostics are `check` of the buffer: its bodies and closers as typed,
//! unsaved, and its inputs as they are on disk. They are published when a
//! document opens or changes, and for every open document when any is
//! saved, since a saved file may be another template's input. Hover on a
//! region's lines shows its loader, opener, state, inputs and sums. A code
//! lens on each opener runs the file: the server renders the buffer as
//! `run` would and hands the editor the new text as an edit, so an unsaved
//! buffer is rendered as it stands and the change can be undone. Trust is
//! the store's, as for `run`: an untrusted exec region is reported and not
//! run.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    CodeLens, CodeLensOptions, CodeLensParams, Command, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    ExecuteCommandOptions, ExecuteCommandParams, Hover, HoverContents, HoverParams,
    HoverProviderCapability, MarkupContent, MarkupKind, MessageType, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, ShowMessageParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri, WorkspaceEdit,
};

use crate::allow;
use crate::guard;
use crate::loader::{Ctx, Production};
use crate::marker::{self, Region, Segment, Syntax};
use crate::render::{self, Action, Loaders, Mode, RegionReport, Rendered, State};
use crate::trust::{self, Store};

/// The command a code lens runs.
pub const RUN: &str = "computed.run";

/// How many input files hover lists before it counts the rest.
const HOVER_INPUTS: usize = 10;

/// Serves the protocol on stdin and stdout until the client exits.
pub fn main() -> Result<u8, String> {
    let (conn, io) = Connection::stdio();
    let store = Store::at(Store::default_path().map_err(|e| e.to_string())?);
    let allow = allow::Store::at(allow::Store::default_path().map_err(|e| e.to_string())?);
    serve(&conn, store, allow)?;
    drop(conn);
    io.join().map_err(|e| e.to_string())?;
    Ok(0)
}

/// Initialises and answers `conn` until the client shuts down. `store`
/// holds the trust grants and `allow` the url prefixes `remote` regions may
/// fetch under, both read as `run` reads them.
pub fn serve(conn: &Connection, store: Store, allow: allow::Store) -> Result<(), String> {
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(false),
        }),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec![RUN.to_string()],
            ..ExecuteCommandOptions::default()
        }),
        ..ServerCapabilities::default()
    };
    let caps = serde_json::to_value(capabilities).map_err(|e| e.to_string())?;
    conn.initialize(caps).map_err(|e| e.to_string())?;
    let mut server = Server {
        conn,
        store,
        allow,
        docs: HashMap::new(),
        next_id: 0,
    };
    for message in &conn.receiver {
        match message {
            Message::Request(req) => {
                if conn.handle_shutdown(&req).map_err(|e| e.to_string())? {
                    return Ok(());
                }
                server.request(req);
            }
            Message::Notification(n) => server.notification(n),
            // The answers to our `workspace/applyEdit`: the client reports
            // a failed edit to its user itself.
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// An open document: where it lives on disk and what the editor holds.
struct Doc {
    path: PathBuf,
    text: String,
    version: Option<i32>,
}

struct Server<'c> {
    conn: &'c Connection,
    store: Store,
    allow: allow::Store,
    docs: HashMap<String, Doc>,
    next_id: u64,
}

/// A region as the server describes it: where it sits and what `check`
/// says of it.
struct Described<'a> {
    region: &'a Region,
    /// 0-based line of the closer.
    closer: u32,
    /// For a region inside a line, the UTF-16 columns its markers and body
    /// span on it.
    within: Option<(u32, u32)>,
    report: Option<&'a RegionReport>,
}

fn regions(parsed: &marker::File) -> Vec<&Region> {
    parsed
        .segments
        .iter()
        .filter_map(|s| match s {
            Segment::Region(r) => Some(r),
            Segment::Prose(_) => None,
        })
        .collect()
}

fn describe<'a>(
    parsed: &'a marker::File,
    reports: &'a [RegionReport],
    text: &str,
) -> Vec<Described<'a>> {
    let utf16 = |s: &str| s.encode_utf16().count() as u32;
    regions(parsed)
        .into_iter()
        .map(|region| Described {
            region,
            closer: region.last_line() as u32 - 1,
            within: region.column.map(|column| {
                let line = text.split('\n').nth(region.line - 1).unwrap_or("");
                let before: String = line.chars().take(column - 1).collect();
                let start = utf16(&before);
                let whole = format!("{}{}{}", region.raw_opener, region.body, region.raw_closer);
                (start, start + utf16(&whole))
            }),
            report: reports.iter().find(|r| (r.line, r.column) == region.at()),
        })
        .collect()
}

fn range(d: &Described<'_>) -> Range {
    let line = d.region.line as u32 - 1;
    if let Some((start, end)) = d.within {
        return Range::new(Position::new(line, start), Position::new(line, end));
    }
    let closer = d.region.raw_closer.trim_end_matches(['\n', '\r']);
    Range::new(
        Position::new(d.region.line as u32 - 1, 0),
        Position::new(d.closer, closer.encode_utf16().count() as u32),
    )
}

/// The path of a `file:` URI.
fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let mut bytes = Vec::with_capacity(path.len());
    let mut it = path.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex = [it.next()?, it.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}

/// The `file:` URI of an absolute path.
pub fn uri_of(path: &Path) -> Uri {
    let mut out = String::from("file://");
    for &b in path.to_string_lossy().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            write!(out, "%{b:02X}").unwrap();
        }
    }
    Uri::from_str(&out).expect("a percent-encoded file URI parses")
}

impl Server<'_> {
    fn send(&self, message: impl Into<Message>) {
        // The client has gone when this fails; the receive loop ends next.
        let _ = self.conn.sender.send(message.into());
    }

    fn notify<P: serde::Serialize>(&self, method: &str, params: P) {
        self.send(Notification::new(method.to_string(), params));
    }

    fn show(&self, typ: MessageType, message: String) {
        self.notify("window/showMessage", ShowMessageParams { typ, message });
    }

    fn trusted(&self, path: &Path) -> bool {
        let ctx = Ctx::for_template(path);
        let root = match ctx.repo_root {
            Some(r) => r,
            None => match trust::root_for(&ctx.region_root) {
                Ok(r) => r,
                Err(_) => return false,
            },
        };
        self.store.is_trusted(&root).unwrap_or(false)
    }

    fn notification(&mut self, n: Notification) {
        match n.method.as_str() {
            "textDocument/didOpen" => {
                let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(n.params) else {
                    return;
                };
                let key = p.text_document.uri.as_str().to_string();
                let Some(path) = path_of(&key) else { return };
                self.docs.insert(
                    key.clone(),
                    Doc {
                        path,
                        text: p.text_document.text,
                        version: Some(p.text_document.version),
                    },
                );
                self.publish(&key);
            }
            "textDocument/didChange" => {
                let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(n.params) else {
                    return;
                };
                let key = p.text_document.uri.as_str().to_string();
                let (Some(doc), Some(change)) = (
                    self.docs.get_mut(&key),
                    p.content_changes.into_iter().last(),
                ) else {
                    return;
                };
                doc.text = change.text;
                doc.version = Some(p.text_document.version);
                self.publish(&key);
            }
            // A saved file may be any open template's input.
            "textDocument/didSave" => {
                let keys: Vec<String> = self.docs.keys().cloned().collect();
                for key in keys {
                    self.publish(&key);
                }
            }
            "textDocument/didClose" => {
                let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(n.params) else {
                    return;
                };
                self.docs.remove(p.text_document.uri.as_str());
                self.notify(
                    "textDocument/publishDiagnostics",
                    PublishDiagnosticsParams::new(p.text_document.uri, Vec::new(), None),
                );
            }
            _ => {}
        }
    }

    fn request(&mut self, req: Request) {
        let id = req.id.clone();
        let result = match req.method.as_str() {
            "textDocument/hover" => serde_json::from_value::<HoverParams>(req.params)
                .map(|p| serde_json::to_value(self.hover(&p)).unwrap()),
            "textDocument/codeLens" => serde_json::from_value::<CodeLensParams>(req.params)
                .map(|p| serde_json::to_value(self.lenses(p.text_document.uri.as_str())).unwrap()),
            "workspace/executeCommand" => {
                serde_json::from_value::<ExecuteCommandParams>(req.params).map(|p| {
                    self.execute(&p);
                    serde_json::Value::Null
                })
            }
            _ => {
                self.send(Response::new_err(
                    id,
                    lsp_server::ErrorCode::MethodNotFound as i32,
                    format!("computed lsp does not answer {}", req.method),
                ));
                return;
            }
        };
        self.send(match result {
            Ok(value) => Response::new_ok(id, value),
            Err(e) => Response::new_err(
                id,
                lsp_server::ErrorCode::InvalidParams as i32,
                e.to_string(),
            ),
        });
    }

    fn publish(&self, key: &str) {
        let Some(doc) = self.docs.get(key) else {
            return;
        };
        let diagnostics = self.diagnostics(doc);
        let Ok(uri) = Uri::from_str(key) else { return };
        self.notify(
            "textDocument/publishDiagnostics",
            PublishDiagnosticsParams::new(uri, diagnostics, doc.version),
        );
    }

    fn diagnostics(&self, doc: &Doc) -> Vec<Diagnostic> {
        if !marker::has_marker(&doc.text, Syntax::for_path(&doc.path)) {
            return Vec::new();
        }
        let reports = match guard::check_text(&doc.path, &doc.text) {
            Ok(r) => r,
            Err((line, message)) => {
                let at = Position::new(line as u32 - 1, 0);
                return vec![Diagnostic {
                    range: Range::new(at, Position::new(line as u32, 0)),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("computed".into()),
                    message,
                    ..Diagnostic::default()
                }];
            }
        };
        let Ok(parsed) = marker::parse_as(&doc.text, Syntax::for_path(&doc.path)) else {
            return Vec::new();
        };
        let trusted = self.trusted(&doc.path);
        describe(&parsed, &reports, &doc.text)
            .iter()
            .filter_map(|d| {
                let report = d.report?;
                let source = guard::source(d.region);
                let (severity, message) = match report.state {
                    State::Fresh | State::Volatile => return None,
                    s if render::needs_trust(&report.loader) && !trusted && s != State::Error => (
                        DiagnosticSeverity::INFORMATION,
                        format!(
                            "untrusted: {s}, and this clone does not run exec or transcript regions; `computed trust` lets it ({source})"
                        ),
                    ),
                    State::Stale => (
                        DiagnosticSeverity::WARNING,
                        format!("stale: inputs or opener changed since render ({source})"),
                    ),
                    State::Unrendered => (
                        DiagnosticSeverity::WARNING,
                        format!("unrendered: never rendered ({source})"),
                    ),
                    State::Edited => (
                        DiagnosticSeverity::ERROR,
                        format!(
                            "edited: the body was changed by hand, and `run` refuses the file ({source})"
                        ),
                    ),
                    State::StaleEdited => (
                        DiagnosticSeverity::ERROR,
                        format!(
                            "stale+edited: inputs changed and the body was changed by hand ({source})"
                        ),
                    ),
                    State::Error => (
                        DiagnosticSeverity::ERROR,
                        format!(
                            "error: {} ({source})",
                            report.stderr.as_deref().unwrap_or("the tool cannot answer")
                        ),
                    ),
                };
                Some(Diagnostic {
                    range: range(d),
                    severity: Some(severity),
                    source: Some("computed".into()),
                    message,
                    ..Diagnostic::default()
                })
            })
            .collect()
    }

    fn hover(&self, p: &HoverParams) -> Option<Hover> {
        let doc = self
            .docs
            .get(p.text_document_position_params.text_document.uri.as_str())?;
        let at = p.text_document_position_params.position;
        let mut parsed = marker::parse_as(&doc.text, Syntax::for_path(&doc.path)).ok()?;
        let mut loaders = Production::for_file(&doc.path, &mut parsed);
        let reports = guard::check_text(&doc.path, &doc.text).unwrap_or_default();
        let described = describe(&parsed, &reports, &doc.text);
        let d = described.iter().find(|d| {
            let r = range(d);
            (r.start.line..=r.end.line).contains(&at.line)
                && d.within
                    .is_none_or(|(start, end)| (start..=end).contains(&at.character))
        })?;
        let region = d.region;
        let state = d
            .report
            .map_or("unknown".to_string(), |r| r.state.to_string());
        let mut md = format!(
            "**computed** `{}` region{}: **{state}**\n\n```\n{}\n```\n",
            region.opener.loader,
            region
                .opener
                .name
                .as_ref()
                .map_or(String::new(), |n| format!(" `{n}`")),
            region.opener.canonical()
        );
        if let Some(written) = region.opener.written() {
            writeln!(md, "\nExpanded from `{written}` by its recipe.").unwrap();
        }
        // The files this region's snapshot reads, from a snapshot of its own.
        let snapshot = loaders.snapshot(region);
        let root = Ctx::for_template(&doc.path)
            .region_root
            .canonicalize()
            .unwrap_or_default();
        let inputs: Vec<String> = loaders
            .read()
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        md.push_str("\n**Inputs:** ");
        match (&snapshot, region.opener.loader.as_str()) {
            (Err(_), _) => md.push_str("cannot be read\n"),
            (Ok(None), _) => md.push_str("none, the region is volatile\n"),
            (Ok(Some(_)), "tree") => writeln!(
                md,
                "the listing of `{}`",
                region.opener.attr("src").unwrap_or(".")
            )
            .unwrap(),
            (Ok(Some(_)), _) if inputs.is_empty() => md.push_str("none\n"),
            (Ok(Some(_)), _) => {
                md.push('\n');
                for i in inputs.iter().take(HOVER_INPUTS) {
                    writeln!(md, "- `{i}`").unwrap();
                }
                if inputs.len() > HOVER_INPUTS {
                    writeln!(md, "- and {} more", inputs.len() - HOVER_INPUTS).unwrap();
                }
            }
        }
        match &region.sums {
            Some(s) => write!(
                md,
                "\n**Sums:** `in={}…` `out={}…`",
                &s.input[..12],
                &s.output[..12]
            )
            .unwrap(),
            None => md.push_str("\n**Sums:** none, never rendered"),
        }
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: md,
            }),
            range: Some(range(d)),
        })
    }

    fn lenses(&self, key: &str) -> Vec<CodeLens> {
        let Some(doc) = self.docs.get(key) else {
            return Vec::new();
        };
        let Ok(parsed) = marker::parse_as(&doc.text, Syntax::for_path(&doc.path)) else {
            return Vec::new();
        };
        let reports = guard::check_text(&doc.path, &doc.text).unwrap_or_default();
        describe(&parsed, &reports, &doc.text)
            .iter()
            .map(|d| {
                let state = d
                    .report
                    .map_or("unknown".to_string(), |r| r.state.to_string());
                let at = range(d).start;
                CodeLens {
                    range: Range::new(at, at),
                    command: Some(Command {
                        title: format!("computed: {state} — Run"),
                        command: RUN.to_string(),
                        arguments: Some(vec![serde_json::Value::String(key.to_string())]),
                    }),
                    data: None,
                }
            })
            .collect()
    }

    /// `computed.run` on the document the first argument names: renders the
    /// buffer as `run` would, and asks the editor to apply the result.
    fn execute(&mut self, p: &ExecuteCommandParams) {
        if p.command != RUN {
            return;
        }
        let Some(key) = p.arguments.first().and_then(|a| a.as_str()) else {
            return;
        };
        let Some(doc) = self.docs.get(key) else {
            self.show(
                MessageType::WARNING,
                "computed: open the file to run it".into(),
            );
            return;
        };
        let mut parsed = match marker::parse_as(&doc.text, Syntax::for_path(&doc.path)) {
            Ok(p) => p,
            Err(e) => {
                self.show(
                    MessageType::ERROR,
                    format!("computed: line {}: {}", e.line, e.message),
                );
                return;
            }
        };
        let trusted = self.trusted(&doc.path);
        let allowed = match self.allow.allowed(&[]) {
            Ok(a) => a,
            Err(e) => {
                self.show(MessageType::ERROR, format!("computed: {e}"));
                return;
            }
        };
        let mut loaders = Production::for_file(&doc.path, &mut parsed).allowing(allowed);
        let rendered = render::file(&parsed, Mode::Run { force: false }, trusted, &mut loaders);
        let name = doc
            .path
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let lines: Vec<String> = rendered
            .regions()
            .iter()
            .filter(|r| r.action != Some(Action::Fresh))
            .map(|r| {
                let mut line = format!(
                    "{name}:{} {} {}",
                    marker::place(r.line, r.column),
                    r.name.as_deref().unwrap_or(&r.loader),
                    r.action.map_or(String::new(), |a| a.to_string())
                );
                if let Some(e) = &r.stderr {
                    write!(line, ": {e}").unwrap();
                }
                line
            })
            .collect();
        let (typ, text) = match &rendered {
            Rendered::Error { line, message } => (
                MessageType::ERROR,
                Some(format!("computed: {name}:{line}: {message}")),
            ),
            Rendered::Refused { .. } => (
                MessageType::WARNING,
                Some(format!(
                    "computed: {} (`computed run --force` overwrites the edit)",
                    lines.join("; ")
                )),
            ),
            Rendered::Unchanged { .. } | Rendered::Written { .. } if lines.is_empty() => {
                (MessageType::INFO, None)
            }
            _ => (
                MessageType::INFO,
                Some(format!("computed: {}", lines.join("; "))),
            ),
        };
        if let Some(text) = text {
            self.show(typ, text);
        }
        let Rendered::Written { text: new, .. } = rendered else {
            return;
        };
        let last = doc.text.split('\n').enumerate().last().unwrap_or((0, ""));
        let end = Position::new(last.0 as u32, last.1.encode_utf16().count() as u32);
        let Ok(uri) = Uri::from_str(key) else { return };
        let edit = WorkspaceEdit {
            changes: Some(
                [(
                    uri,
                    vec![TextEdit::new(Range::new(Position::new(0, 0), end), new)],
                )]
                .into(),
            ),
            ..WorkspaceEdit::default()
        };
        self.next_id += 1;
        let id = RequestId::from(format!("computed/{}", self.next_id));
        self.send(Request::new(
            id,
            "workspace/applyEdit".to_string(),
            lsp_types::ApplyWorkspaceEditParams {
                label: Some("computed run".into()),
                edit,
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_round_trip() {
        let path = Path::new("/tmp/a dir/ü#1.md");
        let uri = uri_of(path);
        assert_eq!(uri.as_str(), "file:///tmp/a%20dir/%C3%BC%231.md");
        assert_eq!(path_of(uri.as_str()).unwrap(), path);
        assert_eq!(
            path_of("file://localhost/x/y.md").unwrap(),
            Path::new("/x/y.md")
        );
        assert_eq!(path_of("untitled:1"), None);
    }
}
