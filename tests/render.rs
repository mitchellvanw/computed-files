//! `render` through a fake `Loaders` against golden files under `tests/fixtures`.
//! The fixtures carry a `.txt` suffix so `computed` discovery in this repository skips them.
//! Set `UPDATE_GOLDEN=1` to rewrite the expected files.

use std::collections::HashMap;
use std::path::PathBuf;

use computed::loader::{LoadError, Loaded};
use computed::marker::{self, Region};
use computed::render::{self, Loaders, Mode, Rendered};

/// What the fake answers for one region, keyed by `name=`.
#[derive(Clone)]
enum Entry {
    /// `inputs=`-style loader: a snapshot and the text it would load.
    Inputs {
        snapshot: &'static str,
        text: &'static str,
    },
    /// A volatile region: no snapshot, always this text.
    Volatile(&'static str),
    /// `snapshot` succeeds, `load` fails with this stderr.
    Fails {
        snapshot: &'static str,
        stderr: &'static str,
    },
    /// `snapshot` is a hard error.
    Hard(&'static str),
}

#[derive(Default)]
struct Fake {
    table: HashMap<&'static str, Entry>,
    loads: Vec<String>,
}

impl Fake {
    fn with(entries: &[(&'static str, Entry)]) -> Fake {
        Fake {
            table: entries.iter().cloned().collect(),
            loads: Vec::new(),
        }
    }
    fn entry(&self, region: &Region) -> &Entry {
        let name = region
            .opener
            .name
            .as_deref()
            .expect("fixture regions are named");
        self.table
            .get(name)
            .unwrap_or_else(|| panic!("no fake entry for {name}"))
    }
}

impl Loaders for Fake {
    fn snapshot(&mut self, region: &Region) -> Result<Option<Vec<u8>>, LoadError> {
        match self.entry(region) {
            Entry::Inputs { snapshot, .. } | Entry::Fails { snapshot, .. } => {
                Ok(Some(snapshot.as_bytes().to_vec()))
            }
            Entry::Volatile(_) => Ok(None),
            Entry::Hard(m) => Err(LoadError::Hard(m.to_string())),
        }
    }
    fn load(&mut self, region: &Region) -> Result<Loaded, LoadError> {
        self.loads.push(region.opener.name.clone().unwrap());
        match self.entry(region) {
            Entry::Inputs { snapshot, text } => Ok(Loaded {
                text: text.to_string(),
                snapshot: snapshot.as_bytes().to_vec(),
            }),
            Entry::Volatile(text) => Ok(Loaded {
                text: text.to_string(),
                snapshot: Vec::new(),
            }),
            Entry::Fails { stderr, .. } => Err(LoadError::Failed {
                stderr: stderr.to_string(),
            }),
            Entry::Hard(m) => Err(LoadError::Hard(m.to_string())),
        }
    }
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn golden(name: &str, actual: &str) {
    let path = fixtures().join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        expected == actual,
        "{name} differs from golden\n--- expected\n{expected}\n--- actual\n{actual}"
    );
}

fn read(name: &str) -> String {
    std::fs::read_to_string(fixtures().join(name)).unwrap()
}

/// One report line per region, the shape the golden files fix.
fn report_text(rendered: &Rendered) -> String {
    let mut out = String::new();
    let (label, regions) = match rendered {
        Rendered::Written { regions, .. } => ("written", regions),
        Rendered::Unchanged { regions } => ("unchanged", regions),
        Rendered::Refused { regions } => ("refused", regions),
        Rendered::Error { line, message } => {
            return format!("error line {line}: {message}\ntier 2\n");
        }
    };
    out.push_str(label);
    out.push('\n');
    for r in regions {
        let action = r.action.map(|a| a.to_string()).unwrap_or_default();
        out.push_str(&format!(
            "{} {} {} {} {}\n",
            r.line,
            r.name.as_deref().unwrap_or("-"),
            r.loader,
            r.state,
            action
        ));
        if let Some(stderr) = &r.stderr {
            out.push_str(&format!("  stderr: {stderr}\n"));
        }
    }
    out.push_str(&format!("tier {}\n", rendered.tier()));
    out
}

fn run_case(input: &str, out: &str, mode: Mode, trusted: bool, fake: &mut Fake) -> Rendered {
    let text = read(input);
    let file = marker::parse(&text).unwrap();
    let rendered = render::file(&file, mode, trusted, fake);
    if let Rendered::Written { text, .. } = &rendered {
        golden(&format!("{out}.txt"), text);
    }
    golden(&format!("{out}.report.txt"), &report_text(&rendered));
    rendered
}

fn standard() -> Fake {
    Fake::with(&[
        (
            "layout",
            Entry::Inputs {
                snapshot: "docs/\nsrc/\nsrc/main.rs\n",
                text: ".\n├── docs\n└── src\n    └── main.rs\n",
            },
        ),
        (
            "adrs",
            Entry::Inputs {
                snapshot: "docs/adr/0001.md\x006\x00# One\n\n\0",
                text: "# One\n",
            },
        ),
        (
            "deps",
            Entry::Inputs {
                snapshot: "Cargo.toml\x0010\x00[package]\n\n\0",
                text: "[package]\n",
            },
        ),
        ("now", Entry::Volatile("2026-09-03\n")),
    ])
}

#[test]
fn run_renders_an_unrendered_file() {
    let mut fake = standard();
    let r = run_case(
        "unrendered.in.txt",
        "unrendered.run",
        Mode::Run { force: false },
        true,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Written { .. }));
    assert_eq!(fake.loads, ["layout", "adrs", "deps", "now"]);
}

#[test]
fn run_on_a_fresh_file_is_unchanged_and_loads_only_volatile() {
    let mut fake = standard();
    let r = run_case(
        "fresh.in.txt",
        "fresh.run",
        Mode::Run { force: false },
        true,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Unchanged { .. }));
    assert_eq!(fake.loads, ["now"]);
}

#[test]
fn check_never_loads() {
    let mut fake = standard();
    run_case("fresh.in.txt", "fresh.check", Mode::Check, false, &mut fake);
    assert!(fake.loads.is_empty());
    let mut fake = standard();
    let r = run_case(
        "states.in.txt",
        "states.check",
        Mode::Check,
        false,
        &mut fake,
    );
    assert!(fake.loads.is_empty());
    assert_eq!(r.tier(), 1);
}

#[test]
fn run_refuses_an_edited_region_and_writes_nothing() {
    let mut fake = standard();
    let r = run_case(
        "states.in.txt",
        "states.run",
        Mode::Run { force: false },
        true,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Refused { .. }));
    assert!(
        fake.loads.is_empty(),
        "refuse is decided before any loader runs"
    );
}

#[test]
fn run_force_overwrites_edited_regions() {
    let mut fake = standard();
    let r = run_case(
        "states.in.txt",
        "states.force",
        Mode::Run { force: true },
        true,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Written { .. }));
}

#[test]
fn dry_run_reports_what_run_would_write() {
    let mut fake = standard();
    let r = run_case(
        "states.in.txt",
        "states.dry-run",
        Mode::DryRun { force: true },
        true,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Written { .. }));
}

#[test]
fn untrusted_skips_exec_and_still_renders_tree() {
    let mut fake = standard();
    let r = run_case(
        "unrendered.in.txt",
        "unrendered.untrusted",
        Mode::Run { force: false },
        false,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Written { .. }));
    assert_eq!(fake.loads, ["layout"]);
    assert_eq!(r.tier(), 1);
}

#[test]
fn loader_failure_keeps_the_body_and_sums() {
    let mut fake = Fake::with(&[
        (
            "layout",
            Entry::Inputs {
                snapshot: "docs/\nsrc/\nsrc/main.rs\n",
                text: ".\n├── docs\n└── src\n    └── main.rs\n",
            },
        ),
        (
            "adrs",
            Entry::Fails {
                snapshot: "changed",
                stderr: "grep: no such file\nline two",
            },
        ),
        (
            "deps",
            Entry::Inputs {
                snapshot: "Cargo.toml\x0010\x00[package]\n\n\0",
                text: "[package]\n",
            },
        ),
        ("now", Entry::Volatile("2026-09-03\n")),
    ]);
    let r = run_case(
        "fresh.in.txt",
        "fresh.failure",
        Mode::Run { force: false },
        true,
        &mut fake,
    );
    assert_eq!(r.tier(), 1);
}

#[test]
fn text_that_fails_normalisation_is_a_loader_failure() {
    let mut fake = Fake::with(&[
        (
            "layout",
            Entry::Inputs {
                snapshot: "x",
                text: "```\nunbalanced\n",
            },
        ),
        (
            "adrs",
            Entry::Inputs {
                snapshot: "x",
                text: "<!-- /computed -->\n",
            },
        ),
        (
            "deps",
            Entry::Inputs {
                snapshot: "x",
                text: "a\x00b",
            },
        ),
        ("now", Entry::Volatile("ok\n")),
    ]);
    let r = run_case(
        "unrendered.in.txt",
        "unrendered.normalisation",
        Mode::Run { force: false },
        true,
        &mut fake,
    );
    assert_eq!(r.tier(), 1);
}

#[test]
fn a_changed_file_leaves_an_unchanged_volatile_region_silent() {
    let mut fake = standard();
    fake.table.insert(
        "layout",
        Entry::Inputs {
            snapshot: "moved",
            text: ".\n└── moved\n",
        },
    );
    let r = run_case(
        "fresh.in.txt",
        "fresh.stale-layout",
        Mode::Run { force: false },
        true,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Written { .. }));
    assert_eq!(fake.loads, ["layout", "now"]);
    let now = r
        .regions()
        .iter()
        .find(|r| r.name.as_deref() == Some("now"))
        .unwrap();
    assert_eq!(now.action, Some(render::Action::Fresh));
}

#[test]
fn a_hard_error_from_load_is_the_files_error() {
    struct HardLoad;
    impl Loaders for HardLoad {
        fn snapshot(&mut self, _: &Region) -> Result<Option<Vec<u8>>, LoadError> {
            Ok(None)
        }
        fn load(&mut self, _: &Region) -> Result<Loaded, LoadError> {
            Err(LoadError::Hard("/bin/sh: not found".into()))
        }
    }
    let file = marker::parse(&read("unrendered.in.txt")).unwrap();
    let r = render::file(&file, Mode::Run { force: false }, true, &mut HardLoad);
    assert!(matches!(r, Rendered::Error { line: 5, .. }), "{r:?}");
    assert_eq!(r.tier(), 2);
}

#[test]
fn a_hard_loader_error_skips_the_file() {
    let mut fake = Fake::with(&[
        ("layout", Entry::Hard("src= escapes the repository root")),
        ("adrs", Entry::Volatile("")),
        ("deps", Entry::Volatile("")),
        ("now", Entry::Volatile("")),
    ]);
    let r = run_case(
        "unrendered.in.txt",
        "unrendered.hard",
        Mode::Run { force: false },
        true,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Error { line: 5, .. }));
    assert_eq!(r.tier(), 2);
}

#[test]
fn clean_empties_bodies_and_strips_sums() {
    let mut fake = standard();
    let r = run_case(
        "fresh.in.txt",
        "fresh.clean",
        Mode::Clean {
            force: false,
            dry_run: false,
        },
        false,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Written { .. }));
    assert!(fake.loads.is_empty());
    let mut fake = standard();
    let r = run_case(
        "states.in.txt",
        "states.clean",
        Mode::Clean {
            force: false,
            dry_run: false,
        },
        false,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Refused { .. }));
    let mut fake = standard();
    run_case(
        "states.in.txt",
        "states.clean-force",
        Mode::Clean {
            force: true,
            dry_run: false,
        },
        false,
        &mut fake,
    );
    let mut fake = standard();
    run_case(
        "states.in.txt",
        "states.clean-dry-run",
        Mode::Clean {
            force: true,
            dry_run: true,
        },
        false,
        &mut fake,
    );
    let mut fake = standard();
    let r = run_case(
        "unrendered.in.txt",
        "unrendered.clean",
        Mode::Clean {
            force: false,
            dry_run: false,
        },
        false,
        &mut fake,
    );
    assert!(
        matches!(r, Rendered::Written { .. }),
        "the unrendered region with a stray body is emptied"
    );
    let cleaned = marker::parse(&read("unrendered.clean.txt")).unwrap();
    let r = render::file(
        &cleaned,
        Mode::Clean {
            force: false,
            dry_run: false,
        },
        false,
        &mut fake,
    );
    assert!(matches!(r, Rendered::Unchanged { .. }));
    assert!(fake.loads.is_empty());
}

#[test]
fn a_cleaned_file_renders_back_to_the_fresh_file() {
    let fresh = marker::parse(&read("fresh.in.txt")).unwrap();
    let mut fake = standard();
    let Rendered::Written { text: cleaned, .. } = render::file(
        &fresh,
        Mode::Clean {
            force: false,
            dry_run: false,
        },
        false,
        &mut fake,
    ) else {
        panic!("clean writes");
    };
    let file = marker::parse(&cleaned).unwrap();
    match render::file(&file, Mode::Run { force: false }, true, &mut fake) {
        Rendered::Written { text, .. } => assert_eq!(text, read("fresh.in.txt")),
        other => panic!("{other:?}"),
    }
}
