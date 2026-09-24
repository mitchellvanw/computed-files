//! The `table` sink: CSV, TSV or JSON Lines shaped into a Markdown table.

mod sandbox;

use computed::marker::{self, Sink};
use computed::sink;
use computed::table::TableFrom;
use sandbox::{Sandbox, code, parse_error, region, stderr};

#[test]
fn the_grammar_takes_delim_and_from_only_with_as_table() {
    assert_eq!(
        region("<!-- computed file src=a.tsv as=table delim=tab -->")
            .opener
            .sink,
        Sink::Table(TableFrom::Delimited(b'\t'))
    );
    assert_eq!(
        region("<!-- computed exec cmd=x volatile from=jsonl as=table -->")
            .opener
            .sink,
        Sink::Table(TableFrom::Jsonl)
    );
    let r = region("<!-- computed file src=a.csv as=table -->");
    assert_eq!(r.opener.sink, Sink::Table(TableFrom::Delimited(b',')));
    assert!(r.opener.attrs.iter().all(|(k, _)| k == "src"));
    for (opener, needle) in [
        (
            "<!-- computed file src=a.csv delim=tab -->",
            "unknown attribute delim= for loader file: it applies only with as=table",
        ),
        (
            "<!-- computed file src=a.csv as=fence from=csv -->",
            "unknown attribute from=",
        ),
        (
            "<!-- computed file src=a.csv as=table delim=semicolon -->",
            "delim=semicolon: expected comma or tab",
        ),
        (
            "<!-- computed file src=a.csv as=table from=jsonl delim=tab -->",
            "does not apply",
        ),
        (
            "<!-- computed file src=a.csv as=table from=csv from=csv -->",
            "duplicate attribute from=",
        ),
    ] {
        let m = parse_error(opener);
        assert!(m.contains(needle), "{opener}: {m}");
    }
}

#[test]
fn the_body_is_a_table_between_blank_lines_and_parses_back() {
    let body = sink::body(
        Sink::Table(TableFrom::Delimited(b',')),
        "",
        b"name,note\r\ncomputed,\"<!-- /computed -->\"\r\n```,x\r\n",
    )
    .unwrap();
    assert_eq!(
        body,
        "\n| name     | note               |\n| -------- | ------------------ |\n| computed | <!-- /computed --> |\n| ```      | x                  |\n\n"
    );
    let e = sink::body(Sink::Table(TableFrom::Jsonl), "", b"[1]\n").unwrap_err();
    assert!(e.contains("not a JSON object"), "{e}");
}

#[test]
fn a_csv_file_renders_as_a_table_and_settles() {
    let s = Sandbox::new();
    s.write("data/deps.csv", "crate,version\nclap,4\nsha2,0.11\n")
        .write(
            "README.md",
            "## Dependencies\n\n<!-- computed file src=data/deps.csv as=table name=deps -->\n<!-- /computed -->\n",
        );
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    let rendered = s.read("README.md");
    assert!(
        rendered.contains(
            "-->\n\n| crate | version |\n| ----- | ------- |\n| clap  | 4       |\n| sha2  | 0.11    |\n\n<!-- /computed in="
        ),
        "{rendered}"
    );
    assert!(marker::parse(&rendered).is_ok());
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    assert_eq!(s.read("README.md"), rendered);
}

#[test]
fn a_ragged_row_is_a_loader_failure_that_keeps_the_body() {
    let s = Sandbox::new();
    s.write("data.csv", "a,b\n1\n").write(
        "README.md",
        "<!-- computed file src=data.csv as=table -->\nkept\n<!-- /computed -->\n",
    );
    let out = s.run(&["run"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains("row 2 has 1 field; the header has 2"),
        "{}",
        stderr(&out)
    );
    assert!(s.read("README.md").contains("\nkept\n"));
}

#[test]
fn json_lines_from_a_command() {
    let s = Sandbox::new();
    s.write("rows.jsonl", "{\"k\": \"a|b\", \"n\": 1}\n").write(
        "README.md",
        "<!-- computed exec cmd=\"cat rows.jsonl\" inputs=rows.jsonl as=table from=jsonl -->\n<!-- /computed -->\n",
    );
    let out = s.run(&["run", "--trust"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(
        s.read("README.md")
            .contains("\n| k    | n   |\n| ---- | --- |\n| a\\|b | 1   |\n"),
        "{}",
        s.read("README.md")
    );
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
}
