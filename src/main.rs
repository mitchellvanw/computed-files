//! PROTOTYPE — throwaway code answering one question; see README.md.
//! The computed-markdown sketch, lifted from the in-memory HTML logic prototype onto real files.

mod load;
mod ops;
mod parse;
mod publish;
mod render;
mod report;
mod sink;
mod watch;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

const TMPL: &str = "CLAUDE.md.tmpl";
const OUT: &str = "CLAUDE.md";

const TEMPLATE: &str = r"# Project notes

Hand-written prose stays exactly as typed. The tool only touches regions.

## Layout

<!-- computed tree src=. depth=2 name=layout -->
<!-- /computed -->

## Data files

<!-- computed csv src=data.csv name=data -->
<!-- /computed -->

Run tests with `cargo test`.
";

fn default_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".scratch")
        .join("computed-markdown-proto")
}

fn usage() {
    println!(
        "computed — PROTOTYPE of computed-markdown v1 (throwaway; see README.md)

cargo run -- <command>

  demo                 recreate the scratch demo repo (wipes it)
  run [--force]        render {TMPL} → {OUT}, atomic publish, skip if unchanged
  check                CI mode: exit 1 on drift, never writes
  watch                poll → settle → own-write guard → single-flight render
  clean                delete {OUT}
  cat                  print template and rendered file
  add-file             someone creates src/watcher.rs
  del-file             someone deletes src/lib.rs
  add-row              someone appends a row to data.csv
  rm-csv               someone deletes data.csv (loader failure case)
  edit-region [name]   someone edits inside a region of {OUT}
  edit-prose           someone edits prose outside regions in {OUT}
  add-sh               add a volatile `sh cmd=date` region to the template
  trust | untrust      enable/disable the sh loader for this repo

  --root <dir>         working dir (default .scratch/computed-markdown-proto)"
    );
}

fn done(msg: &str) {
    println!("✓ {}", msg);
    println!("  next: cargo run -- check");
}

fn demo(root: &Path) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join(TMPL), TEMPLATE).unwrap();
    fs::write(root.join("data.csv"), "name,count\ndocs,1\nsrc,2\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn add(a: u32, b: u32) -> u32 { a + b }\n").unwrap();
    fs::write(root.join("docs/notes.md"), "# Notes\n\nScratch file so the tree has depth.\n").unwrap();
    println!(
        "✓ demo repo recreated at {} (no {OUT} yet — run `computed run`)",
        root.display()
    );
}

fn edit_region(root: &Path, name: &str) {
    let out = root.join(OUT);
    let text = match fs::read_to_string(&out) {
        Ok(t) => t,
        Err(_) => return done("nothing to edit: CLAUDE.md does not exist yet"),
    };
    let mut lines: Vec<String> = Vec::new();
    let mut did = false;
    for seg in parse::parse(&text) {
        match seg {
            parse::Seg::Prose(p) => lines.push(p),
            parse::Seg::Error { line, .. } => lines.push(line),
            parse::Seg::Region(r) => {
                let mut body = r.body.clone();
                if !did && (name.is_empty() || r.name == name) {
                    body.push_str("\n<!-- an agent added this line by hand -->");
                    did = true;
                }
                lines.push(format!("{}{}", r.indent, r.open_line));
                lines.push(body);
                let mut c = format!("{}<!-- /computed", r.indent);
                if let Some(s) = &r.in_sum {
                    c.push_str(&format!(" sum={}", s));
                }
                if let Some(s) = &r.out_sum {
                    c.push_str(&format!(" out={}", s));
                }
                c.push_str(" -->");
                lines.push(c);
            }
        }
    }
    if !did {
        return done("no such region to edit");
    }
    fs::write(&out, lines.join("\n")).unwrap();
    done("someone edited a region by hand in CLAUDE.md (sums left as the tool wrote them)");
}

fn edit_prose(root: &Path) {
    let out = root.join(OUT);
    let text = match fs::read_to_string(&out) {
        Ok(t) => t,
        Err(_) => return done("nothing to edit: CLAUDE.md does not exist yet"),
    };
    let text = if text.contains("Run tests with `cargo test`.") {
        text.replace(
            "Run tests with `cargo test`.",
            "Run tests with `cargo test`.\nAlso run `cargo clippy` before pushing.",
        )
    } else {
        format!("{}\nAlso run `cargo clippy` before pushing.", text)
    };
    fs::write(&out, text).unwrap();
    done("someone added a sentence to CLAUDE.md, outside any region");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut root = default_root();
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--root" {
            i += 1;
            match args.get(i) {
                Some(r) => root = PathBuf::from(r),
                None => usage(),
            }
        } else {
            pos.push(args[i].clone());
        }
        i += 1;
    }
    let cmd = pos.first().map(String::as_str).unwrap_or("help");
    let arg = pos.get(1).cloned().unwrap_or_default();

    let tmpl = root.join(TMPL);
    let out = root.join(OUT);
    let need_tmpl = || {
        if !tmpl.exists() {
            eprintln!("no template at {} — run `computed demo` first", tmpl.display());
            exit(2);
        }
    };

    match cmd {
        "help" | "--help" | "-h" => usage(),
        "demo" => demo(&root),
        "run" => {
            need_tmpl();
            let force = arg == "--force";
            let o = ops::run_once(&root, &tmpl, &out, force);
            let kind = if force { "run --force" } else { "run" };
            report::print_report(kind, o.exit, Some(o.wrote), &o.report);
            report::print_output(&out);
            exit(o.exit);
        }
        "check" => {
            need_tmpl();
            match ops::check_once(&root, &tmpl, &out) {
                None => {
                    println!("check → exit 1");
                    println!("  {} is missing", out.display());
                    exit(1);
                }
                Some(o) => {
                    report::print_report("check", o.exit, None, &o.report);
                    exit(o.exit);
                }
            }
        }
        "watch" => {
            need_tmpl();
            watch::watch(&root, &tmpl, &out);
        }
        "clean" => {
            let mut removed = false;
            for p in [out.clone(), root.join(format!("{}.tmp", OUT))] {
                if fs::remove_file(&p).is_ok() {
                    println!("✓ removed {}", p.display());
                    removed = true;
                }
            }
            if !removed {
                done("nothing to clean");
            }
        }
        "cat" => {
            println!("── {} ──", tmpl.display());
            println!("{}", fs::read_to_string(&tmpl).unwrap_or_else(|_| "(missing)".into()));
            report::print_output(&out);
        }
        "add-file" => {
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(root.join("src/watcher.rs"), "pub fn watch() {}\n").unwrap();
            done("created src/watcher.rs (template unchanged — only the input sum can notice)");
        }
        "del-file" => match fs::remove_file(root.join("src/lib.rs")) {
            Ok(_) => done("deleted src/lib.rs"),
            Err(_) => done("src/lib.rs does not exist"),
        },
        "add-row" => {
            let p = root.join("data.csv");
            let text = fs::read_to_string(&p).unwrap();
            let rows = text.lines().count();
            let tail = if text.ends_with('\n') { "" } else { "\n" };
            fs::write(&p, format!("{}tests,{}\n", tail, rows)).unwrap();
            done("appended a row to data.csv");
        }
        "rm-csv" => match fs::remove_file(root.join("data.csv")) {
            Ok(_) => done("deleted data.csv (the csv loader will fail)"),
            Err(_) => done("data.csv does not exist"),
        },
        "edit-region" => edit_region(&root, &arg),
        "edit-prose" => edit_prose(&root),
        "add-sh" => {
            let t = fs::read_to_string(&tmpl).unwrap_or_default();
            if t.contains("computed sh") {
                return done("template already has a sh region");
            }
            fs::write(
                &tmpl,
                format!("{}\n\n## Built at\n\n<!-- computed sh cmd=date name=built -->\n<!-- /computed -->\n", t),
            )
            .unwrap();
            done("added a `sh cmd=date` region to the template (volatile, disabled until trust)");
        }
        "trust" => {
            fs::write(root.join(".computed-trust"), "").unwrap();
            done("shell loader enabled for this repo (.computed-trust present)");
        }
        "untrust" => {
            let _ = fs::remove_file(root.join(".computed-trust"));
            done("shell loader disabled for this repo");
        }
        other => {
            eprintln!("unknown command: {}", other);
            usage();
            exit(2);
        }
    }
}
