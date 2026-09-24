//! `computed lsp` in process, over `Connection::memory()`: diagnostics on
//! open and change, hover, code lens, and the run command.

use std::fs;
use std::path::Path;
use std::time::Duration;

use computed::lsp;
use computed::trust::Store;
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

const TEMPLATE: &str = "# Notes\n\n<!-- computed tree src=src name=layout -->\n<!-- /computed -->\n\n<!-- computed exec cmd=\"echo hi\" volatile name=hi -->\n<!-- /computed -->\n";

struct Client {
    conn: Connection,
    server: Option<std::thread::JoinHandle<Result<(), String>>>,
    next: i32,
    _store: tempfile::TempDir,
}

impl Client {
    fn start() -> Client {
        let (client, server) = Connection::memory();
        let store = tempfile::tempdir().unwrap();
        let path = store.path().join("trust.toml");
        let allow = computed::allow::Store::at(store.path().join("remote.toml"));
        let handle = std::thread::spawn(move || lsp::serve(&server, Store::at(path), allow));
        let mut c = Client {
            conn: client,
            server: Some(handle),
            next: 0,
            _store: store,
        };
        let caps = c.request("initialize", json!({"capabilities": {}}));
        assert!(caps["capabilities"]["hoverProvider"].as_bool().unwrap());
        c.notify("initialized", json!({}));
        c
    }

    fn notify(&self, method: &str, params: Value) {
        self.conn
            .sender
            .send(Notification::new(method.into(), params).into())
            .unwrap();
    }

    /// Sends a request and returns its result, collecting what the server
    /// sends first.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = RequestId::from(self.next);
        self.conn
            .sender
            .send(Request::new(id.clone(), method.into(), params).into())
            .unwrap();
        loop {
            match self.recv() {
                Message::Response(r) if r.id == id => return r.response_result.unwrap(),
                _ => {}
            }
        }
    }

    fn recv(&self) -> Message {
        self.conn
            .receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the server answers")
    }

    /// The next notification with this method, skipping others.
    fn next_notification(&self, method: &str) -> Value {
        loop {
            if let Message::Notification(n) = self.recv()
                && n.method == method
            {
                return n.params;
            }
        }
    }

    fn stop(mut self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        self.server.take().unwrap().join().unwrap().unwrap();
    }
}

fn repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path().canonicalize().unwrap();
    fs::create_dir_all(r.join(".git")).unwrap();
    fs::create_dir_all(r.join("src")).unwrap();
    fs::write(r.join("src/main.rs"), "").unwrap();
    let notes = r.join("NOTES.md");
    fs::write(&notes, TEMPLATE).unwrap();
    (dir, notes)
}

fn uri(path: &Path) -> String {
    lsp::uri_of(path).as_str().to_string()
}

fn open(c: &Client, path: &Path, text: &str) -> Value {
    c.notify(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": uri(path), "languageId": "markdown", "version": 1, "text": text}}),
    );
    c.next_notification("textDocument/publishDiagnostics")
}

#[test]
fn diagnostics_follow_the_buffer() {
    let (_dir, notes) = repo();
    let c = Client::start();
    let published = open(&c, &notes, TEMPLATE);
    assert_eq!(published["uri"], uri(&notes));
    let diags = published["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert_eq!(diags[0]["severity"], 2, "unrendered is a warning");
    assert_eq!(
        diags[0]["message"],
        "unrendered: never rendered (tree src=src name=layout)"
    );
    assert_eq!(
        diags[0]["range"],
        json!({"start": {"line": 2, "character": 0}, "end": {"line": 3, "character": 18}})
    );
    assert_eq!(
        diags[1]["severity"], 3,
        "an untrusted exec region is information"
    );
    assert!(
        diags[1]["message"]
            .as_str()
            .unwrap()
            .starts_with("untrusted: unrendered"),
        "{diags:?}"
    );

    // An unsaved hand edit inside a rendered body is an error.
    let rendered = computed_run(&notes);
    c.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": uri(&notes), "version": 2}, "contentChanges": [{"text": rendered}]}),
    );
    let published = c.next_notification("textDocument/publishDiagnostics");
    assert_eq!(published["diagnostics"], json!([]), "rendered and fresh");
    let edited = rendered.replace("└── main.rs", "└── hand.rs");
    c.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": uri(&notes), "version": 3}, "contentChanges": [{"text": edited}]}),
    );
    let published = c.next_notification("textDocument/publishDiagnostics");
    assert_eq!(published["version"], 3);
    let diags = published["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0]["severity"], 1);
    assert!(
        diags[0]["message"].as_str().unwrap().starts_with("edited:"),
        "{diags:?}"
    );

    // An input on disk moves: stale once any document is saved.
    c.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": uri(&notes), "version": 4}, "contentChanges": [{"text": rendered}]}),
    );
    let published = c.next_notification("textDocument/publishDiagnostics");
    assert_eq!(published["diagnostics"], json!([]));
    fs::write(notes.parent().unwrap().join("src/lib.rs"), "").unwrap();
    c.notify(
        "textDocument/didSave",
        json!({"textDocument": {"uri": uri(&notes)}}),
    );
    let published = c.next_notification("textDocument/publishDiagnostics");
    assert!(
        published["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .starts_with("stale:"),
        "{published}"
    );
    c.stop();
}

#[test]
fn a_parse_error_is_one_diagnostic_at_its_line() {
    let (_dir, notes) = repo();
    let c = Client::start();
    let published = open(&c, &notes, "x\n<!-- computed tree -->\n");
    let diags = published["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["range"]["start"]["line"], 1);
    assert_eq!(diags[0]["severity"], 1);
    c.stop();
}

#[test]
fn hover_and_lenses_describe_the_region() {
    let (_dir, notes) = repo();
    let mut c = Client::start();
    open(&c, &notes, TEMPLATE);
    let hover = c.request(
        "textDocument/hover",
        json!({"textDocument": {"uri": uri(&notes)}, "position": {"line": 3, "character": 2}}),
    );
    let value = hover["contents"]["value"].as_str().unwrap();
    assert!(
        value.contains("`tree` region `layout`: **unrendered**"),
        "{value}"
    );
    assert!(value.contains("the listing of `src`"), "{value}");
    let none = c.request(
        "textDocument/hover",
        json!({"textDocument": {"uri": uri(&notes)}, "position": {"line": 0, "character": 0}}),
    );
    assert_eq!(none, Value::Null, "prose has no hover");

    let lenses = c.request(
        "textDocument/codeLens",
        json!({"textDocument": {"uri": uri(&notes)}}),
    );
    let lenses = lenses.as_array().unwrap();
    assert_eq!(lenses.len(), 2);
    assert_eq!(lenses[0]["range"]["start"]["line"], 2);
    assert_eq!(lenses[0]["command"]["title"], "computed: unrendered — Run");
    assert_eq!(lenses[0]["command"]["command"], lsp::RUN);
    c.stop();
}

#[test]
fn run_hands_the_editor_the_rendered_buffer() {
    let (_dir, notes) = repo();
    let mut c = Client::start();
    open(&c, &notes, TEMPLATE);
    c.next += 1;
    let id = RequestId::from(c.next);
    c.conn
        .sender
        .send(
            Request::new(
                id.clone(),
                "workspace/executeCommand".into(),
                json!({"command": lsp::RUN, "arguments": [uri(&notes)]}),
            )
            .into(),
        )
        .unwrap();
    let mut shown = None;
    let mut edit = None;
    while shown.is_none() || edit.is_none() {
        match c.recv() {
            Message::Notification(n) if n.method == "window/showMessage" => {
                shown = Some(n.params["message"].as_str().unwrap().to_string());
            }
            Message::Request(r) if r.method == "workspace/applyEdit" => {
                edit = Some(r.params.clone());
                c.conn
                    .sender
                    .send(Response::new_ok(r.id, json!({"applied": true})).into())
                    .unwrap();
            }
            _ => {}
        }
    }
    let shown = shown.unwrap();
    assert!(shown.contains("NOTES.md:3 layout written"), "{shown}");
    assert!(
        shown.contains("NOTES.md:6 hi skipped; run `computed trust`"),
        "untrusted exec regions are reported, not run: {shown}"
    );
    let edit = edit.unwrap();
    let edits = &edit["edit"]["changes"][uri(&notes)];
    let text = edits[0]["newText"].as_str().unwrap();
    assert!(text.contains("└── main.rs"), "{text}");
    assert!(!text.contains("hi\n"), "{text}");
    assert_eq!(edits[0]["range"]["end"], json!({"line": 7, "character": 0}));
    assert_eq!(
        fs::read_to_string(&notes).unwrap(),
        TEMPLATE,
        "the disk is the editor's to write"
    );
    c.stop();
}

/// The file as `computed run` renders it, the disk left as it was.
fn computed_run(path: &Path) -> String {
    use assert_cmd::prelude::*;
    let before = fs::read_to_string(path).unwrap();
    let config = tempfile::tempdir().unwrap();
    std::process::Command::cargo_bin("computed")
        .unwrap()
        .env("XDG_CONFIG_HOME", config.path())
        .args(["run", "--trust"])
        .arg(path)
        .output()
        .unwrap();
    let after = fs::read_to_string(path).unwrap();
    fs::write(path, before).unwrap();
    after
}

#[test]
fn a_use_region_is_checked_hovered_and_run_as_its_recipe() {
    let (_dir, notes) = repo();
    let root = notes.parent().unwrap();
    fs::write(
        root.join("computed.toml"),
        "[recipe.layout]\nloader = \"tree\"\nsrc = \"src\"\n",
    )
    .unwrap();
    let template =
        "# Notes\n\n<!-- computed use recipe=layout name=layout -->\n<!-- /computed -->\n";
    fs::write(&notes, template).unwrap();
    let mut c = Client::start();
    let published = open(&c, &notes, template);
    let diags = published["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(
        diags[0]["message"],
        "unrendered: never rendered (use recipe=layout name=layout)"
    );

    let hover = c.request(
        "textDocument/hover",
        json!({"textDocument": {"uri": uri(&notes)}, "position": {"line": 2, "character": 2}}),
    );
    let value = hover["contents"]["value"].as_str().unwrap();
    assert!(
        value.contains("`tree` region `layout`: **unrendered**"),
        "{value}"
    );
    assert!(
        value.contains("Expanded from `<!-- computed use recipe=layout name=layout -->`"),
        "{value}"
    );

    c.next += 1;
    let id = RequestId::from(c.next);
    c.conn
        .sender
        .send(
            Request::new(
                id,
                "workspace/executeCommand".into(),
                json!({"command": lsp::RUN, "arguments": [uri(&notes)]}),
            )
            .into(),
        )
        .unwrap();
    let text = loop {
        if let Message::Request(r) = c.recv()
            && r.method == "workspace/applyEdit"
        {
            let text = r.params["edit"]["changes"][uri(&notes)][0]["newText"]
                .as_str()
                .unwrap()
                .to_string();
            c.conn
                .sender
                .send(Response::new_ok(r.id, json!({"applied": true})).into())
                .unwrap();
            break text;
        }
    };
    assert!(
        text.contains(
            "<!-- computed use recipe=layout name=layout | do not edit; run computed -->"
        ),
        "{text}"
    );
    assert!(text.contains("└── main.rs"), "{text}");
    c.stop();
}

mod sandbox;

/// `computed.run` on `path`: the message shown and the text the editor is
/// asked to apply, if any.
fn run_command(c: &mut Client, path: &Path) -> (String, Option<String>) {
    c.next += 1;
    let id = RequestId::from(c.next);
    c.conn
        .sender
        .send(
            Request::new(
                id.clone(),
                "workspace/executeCommand".into(),
                json!({"command": lsp::RUN, "arguments": [uri(path)]}),
            )
            .into(),
        )
        .unwrap();
    let (mut shown, mut edit) = (String::new(), None);
    loop {
        match c.recv() {
            Message::Response(r) if r.id == id => return (shown, edit),
            Message::Notification(n) if n.method == "window/showMessage" => {
                shown = n.params["message"].as_str().unwrap().to_string();
            }
            Message::Request(r) if r.method == "workspace/applyEdit" => {
                edit = Some(
                    r.params["edit"]["changes"][uri(path)][0]["newText"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                );
                c.conn
                    .sender
                    .send(Response::new_ok(r.id, json!({"applied": true})).into())
                    .unwrap();
            }
            _ => {}
        }
    }
}

#[test]
fn run_fetches_a_remote_region_only_under_the_allowlist() {
    let (_dir, notes) = repo();
    let base = sandbox::serve("fetched\n");
    let template = format!(
        "<!-- computed remote url={base}/doc.md sha256={} name=doc -->\n<!-- /computed -->\n",
        sandbox::pin("fetched\n")
    );
    fs::write(&notes, &template).unwrap();
    let (client, server) = Connection::memory();
    let store = tempfile::tempdir().unwrap();
    let trust = Store::at(store.path().join("trust.toml"));
    let allow = computed::allow::Store::at(store.path().join("remote.toml"));
    let handle = std::thread::spawn(move || lsp::serve(&server, trust, allow));
    let mut c = Client {
        conn: client,
        server: Some(handle),
        next: 0,
        _store: tempfile::tempdir().unwrap(),
    };
    c.request("initialize", json!({"capabilities": {}}));
    c.notify("initialized", json!({}));
    open(&c, &notes, &template);

    let (shown, edit) = run_command(&mut c, &notes);
    assert!(
        shown.contains("doc skipped; run `computed allow`"),
        "{shown}"
    );
    assert!(edit.is_none_or(|t| !t.contains("fetched")));

    computed::allow::Store::at(store.path().join("remote.toml"))
        .allow(&format!("{base}/"))
        .unwrap();
    let (shown, edit) = run_command(&mut c, &notes);
    assert!(shown.contains("doc written"), "{shown}");
    assert!(edit.unwrap().contains("\nfetched\n"));
    c.stop();
}

#[test]
fn a_region_inside_a_line_has_a_range_of_its_own_and_one_in_rust_is_read_too() {
    let (dir, _) = repo();
    let r = dir.path().canonicalize().unwrap();
    fs::write(r.join("v.txt"), "1\n").unwrap();
    let doc = r.join("V.md");
    let text = "ü <!-- computed file src=v.txt name=a -->x<!-- /computed --> <!-- computed file src=v.txt name=b -->y<!-- /computed -->\n";
    fs::write(&doc, text).unwrap();
    let mut c = Client::start();
    let published = open(&c, &doc, text);
    let diags = published["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert_eq!(
        diags[1]["range"],
        json!({"start": {"line": 0, "character": 61}, "end": {"line": 0, "character": 119}})
    );
    let hover = c.request(
        "textDocument/hover",
        json!({"textDocument": {"uri": uri(&doc)}, "position": {"line": 0, "character": 70}}),
    );
    let md = hover["contents"]["value"].as_str().unwrap();
    assert!(md.contains("region `b`"), "{md}");
    let lib = r.join("src/lib.rs");
    let rust = "// computed tree src=.\n// /computed\n";
    fs::write(&lib, rust).unwrap();
    let published = open(&c, &lib, rust);
    let diags = published["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("unrendered")
    );
    c.stop();
}
