//! Regions in files other than Markdown: markers in the file's own comment
//! syntax, `as=comment`, and the `[discover]` table that opts code in.

mod sandbox;

use sandbox::{Sandbox, code, stderr};

const SUMS: &str = "in=0000000000000000000000000000000000000000000000000000000000000000 out=0000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn a_rust_doc_comment_holds_a_tree_behind_its_leader() {
    let s = Sandbox::new();
    s.write("src/a.rs", "").write(
        "src/lib.rs",
        "//! The crate.\n//!\n//! computed tree src=. as=comment lang=text\n//! /computed\n\npub mod a;\n",
    );
    let out = s.run(&["run", "src/lib.rs"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    let text = s.read("src/lib.rs");
    let (head, closer) = text.split_once("//! /computed in=").unwrap();
    assert_eq!(
        head,
        "//! The crate.\n//!\n//! computed tree src=. as=comment lang=text | do not edit; run computed\n//! ```text\n//! .\n//! ├── a.rs\n//! └── lib.rs\n//! ```\n"
    );
    assert!(closer.ends_with("\n\npub mod a;\n"), "{closer}");
    let out = s.run(&["check", "src/lib.rs"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    let out = s.run(&["run", "src/lib.rs"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    s.write("src/b.rs", "");
    let out = s.run(&["check", "src/lib.rs"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains("src/lib.rs:3  tree stale"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn outside_markdown_a_tree_is_raw_and_clean_writes_the_files_own_closer() {
    let s = Sandbox::new();
    s.write("etc/a.conf", "").write(
        "etc/build.sh",
        "#!/bin/sh\n# computed tree src=.\n# /computed\necho done\n",
    );
    let out = s.run(&["run", "etc/build.sh"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    let text = s.read("etc/build.sh");
    assert!(
        text.starts_with("#!/bin/sh\n# computed tree src=. | do not edit; run computed\n\n.\n├── a.conf\n└── build.sh\n\n# /computed in="),
        "{text}"
    );
    let out = s.run(&["clean", "etc/build.sh"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert_eq!(
        s.read("etc/build.sh"),
        "#!/bin/sh\n# computed tree src=. | do not edit; run computed\n# /computed\necho done\n"
    );
}

#[test]
fn a_line_that_would_close_the_region_fails_the_loader() {
    let s = Sandbox::new();
    s.write("notes.txt", "# /computed\n")
        .write("a.py", "# computed file src=notes.txt\n# /computed\n");
    let out = s.run(&["run", "a.py"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains("would parse as a marker: # /computed"),
        "{}",
        stderr(&out)
    );
    // Behind the leader it is a comment that says so, and prose.
    s.write(
        "a.py",
        "# computed file src=notes.txt as=comment\n# /computed\n",
    );
    let out = s.run(&["run", "a.py"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(s.read("a.py").contains("\n# # /computed\n# /computed in="));
}

#[test]
fn discovery_reads_code_only_when_computed_toml_lists_it() {
    let s = Sandbox::new();
    s.write("README.md", "# r\n")
        .write("src/lib.rs", "// computed tree src=.\n// /computed\n")
        .write("Makefile", "# computed exec cmd=x volatile\n# /computed\n");
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    s.write(
        "computed.toml",
        "[discover]\nextensions = [\"rs\"]\nnames = [\"Makefile\"]\n",
    );
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(1));
    let err = stderr(&out);
    assert!(err.contains("Makefile:1  exec unrendered"), "{err}");
    assert!(err.contains("src/lib.rs:1  tree unrendered"), "{err}");
    s.write("computed.toml", "[discover]\nextensions = [\"txt\"]\n");
    let out = s.run(&["check"]);
    assert_eq!(code(&out), Some(2));
    assert!(
        stderr(&out).contains(
            ".txt has no comment syntax the tool knows; it knows the extensions md markdown html"
        ),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_file_region_reads_another_syntaxs_template_without_its_sums() {
    let s = Sandbox::new();
    s.write("src/a.rs", "").write(
        "src/lib.rs",
        "// computed tree src=.\n// /computed\npub mod a;\n",
    );
    s.write(
        "README.md",
        "<!-- computed file src=src/lib.rs as=fence lang=rust -->\n<!-- /computed -->\n",
    );
    let out = s.run(&["run", "README.md", "src/lib.rs"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    let readme = s.read("README.md");
    assert!(
        readme.contains("\n// /computed\npub mod a;\n```\n"),
        "{readme}"
    );
    let out = s.run(&["check", "README.md", "src/lib.rs"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
}

#[test]
fn every_command_reads_a_file_in_its_syntax() {
    let s = Sandbox::new();
    s.write("a.txt", "x\n").write(
        "q.sql",
        &format!("-- computed file src=a.txt name=a\n-- body\n-- /computed {SUMS}\n"),
    );
    let out = s.run(&["stats", "q.sql"]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).starts_with("q.sql:1 a file 1 line 8 bytes"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let out = s.run(&["affected", "a.txt"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    s.write("computed.toml", "[discover]\nextensions = [\"sql\"]\n");
    let out = s.run(&["affected", "a.txt"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "q.sql:1 a file\n");
    let out = s.run(&["check", "q.sql"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        stderr(&out).contains("q.sql:1 a file stale+edited"),
        "{}",
        stderr(&out)
    );
    s.write(
        "proposed.sql",
        "-- computed file src=a.txt name=a\n-- mine\n-- /computed\n",
    );
    let out = s.run(&["guard", "q.sql", "--proposed", "proposed.sql"]);
    assert_eq!(code(&out), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("q.sql:1 a file body+closer changed"),
        "{}",
        stderr(&out)
    );
}
