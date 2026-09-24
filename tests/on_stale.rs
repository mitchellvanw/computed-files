//! `on-stale=warn`: under `check`, pure staleness is reported and does not
//! raise the exit code; every other drift still fails, and `run` is unchanged.

use std::collections::HashMap;

use computed::loader::{LoadError, Loaded};
use computed::marker::{self, OnStale, Region, Segment};
use computed::render::{self, Action, Loaders, Mode, Rendered, State};
use computed::report;

/// Answers every region with the snapshot and text its `name=` maps to.
struct Fake(HashMap<&'static str, (&'static str, &'static str)>);

impl Fake {
    fn get(&self, region: &Region) -> (&'static str, &'static str) {
        self.0[region.opener.name.as_deref().expect("named")]
    }
}

impl Loaders for Fake {
    fn snapshot(&mut self, region: &Region) -> Result<Option<Vec<u8>>, LoadError> {
        Ok(Some(self.get(region).0.as_bytes().to_vec()))
    }
    fn load(&mut self, region: &Region) -> Result<Loaded, LoadError> {
        let (snapshot, text) = self.get(region);
        Ok(Loaded {
            text: text.to_string(),
            snapshot: snapshot.as_bytes().to_vec(),
        })
    }
}

const TEMPLATE: &str = "<!-- computed exec cmd=a inputs=a name=soft on-stale=warn -->\n<!-- /computed -->\n\n<!-- computed exec cmd=b inputs=b name=hard -->\n<!-- /computed -->\n";

/// The template rendered once against `snapshot`, so both regions are fresh.
fn rendered(fake: &mut Fake) -> String {
    let file = marker::parse(TEMPLATE).unwrap();
    match render::file(&file, Mode::Run { force: false }, true, fake) {
        Rendered::Written { text, .. } => text,
        other => panic!("{other:?}"),
    }
}

fn fake(soft: &'static str, hard: &'static str) -> Fake {
    Fake(HashMap::from([
        ("soft", (soft, "a\n")),
        ("hard", (hard, "b\n")),
    ]))
}

fn check(text: &str, fake: &mut Fake) -> Rendered {
    render::file(&marker::parse(text).unwrap(), Mode::Check, false, fake)
}

#[test]
fn the_opener_takes_on_stale_warn_and_nothing_else() {
    let file = marker::parse(TEMPLATE).unwrap();
    let Segment::Region(r) = &file.segments[0] else {
        panic!()
    };
    assert_eq!(r.opener.on_stale, OnStale::Warn);
    assert!(r.opener.canonical().contains("on-stale=warn"));
    assert!(
        r.opener.attr("on-stale").is_none(),
        "a common attribute is lifted out"
    );
    let Segment::Region(r) = &file.segments[2] else {
        panic!()
    };
    assert_eq!(r.opener.on_stale, OnStale::Fail);
    let e =
        marker::parse("<!-- computed tree on-stale=ignore -->\n<!-- /computed -->\n").unwrap_err();
    assert!(e.message.contains("on-stale=ignore"), "{}", e.message);
}

#[test]
fn check_softens_a_stale_region_that_asks_for_it() {
    let text = rendered(&mut fake("1", "1"));
    let r = check(&text, &mut fake("2", "1"));
    let soft = &r.regions()[0];
    assert_eq!(soft.state, State::Stale);
    assert_eq!(soft.action, Some(Action::Warn));
    assert_eq!(r.tier(), 0, "a warned stale region does not fail check");

    let lines = report::regions(
        std::path::Path::new("d.md"),
        r.regions(),
        Mode::Check,
        false,
    );
    assert_eq!(lines, ["d.md:1 soft exec stale warn\n"]);

    let r = check(&text, &mut fake("2", "2"));
    assert_eq!(r.regions()[1].state, State::Stale);
    assert_eq!(r.regions()[1].action, None);
    assert_eq!(r.tier(), 1, "a region without on-stale=warn still fails");
}

#[test]
fn every_other_drift_still_fails() {
    let text = rendered(&mut fake("1", "1"));
    let edited = text.replacen("\na\n", "\nhand\n", 1);
    for (text, snapshot, state) in [
        (edited.as_str(), "1", State::Edited),
        (edited.as_str(), "2", State::StaleEdited),
        (TEMPLATE, "1", State::Unrendered),
    ] {
        let r = check(text, &mut fake(snapshot, "1"));
        let soft = &r.regions()[0];
        assert_eq!(soft.state, state);
        assert_eq!(soft.action, None, "{state}");
        assert_eq!(r.tier(), 1, "{state}");
    }
}

#[test]
fn run_renders_a_stale_warned_region_as_before() {
    let text = rendered(&mut fake("1", "1"));
    let file = marker::parse(&text).unwrap();
    let r = render::file(&file, Mode::Run { force: false }, true, &mut fake("2", "1"));
    assert_eq!(r.regions()[0].action, Some(Action::Written));
    assert_eq!(r.tier(), 1);
}

#[test]
fn json_keeps_action_null_under_check_and_adds_severity() {
    let text = rendered(&mut fake("1", "1"));
    let r = check(&text, &mut fake("2", "1"));
    let doc = report::json(
        &[report::FileJson {
            path: std::path::Path::new("d.md"),
            error: None,
            regions: r.regions(),
            diff: None,
        }],
        0,
    );
    assert!(
        doc.contains("\"state\":\"stale\",\"action\":null,\"message\":null,\"severity\":\"warn\""),
        "{doc}"
    );
    assert!(
        doc.contains("\"name\":\"hard\",\"loader\":\"exec\",\"state\":\"fresh\",\"action\":null,\"message\":null,\"severity\":null"),
        "{doc}"
    );
}

#[test]
fn check_exits_zero_on_a_warned_stale_region_end_to_end() {
    use assert_cmd::prelude::*;
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    std::fs::create_dir(r.join(".git")).unwrap();
    std::fs::write(r.join("part.md"), "one\n").unwrap();
    std::fs::write(
        r.join("doc.md"),
        "<!-- computed file src=part.md name=part on-stale=warn -->\n<!-- /computed -->\n",
    )
    .unwrap();
    let cmd = |args: &[&str]| {
        let mut c = std::process::Command::cargo_bin("computed").unwrap();
        c.current_dir(r)
            .env("XDG_CONFIG_HOME", r.join(".config"))
            .args(args);
        c.output().unwrap()
    };
    assert_eq!(cmd(&["run", "doc.md"]).status.code(), Some(1));
    std::fs::write(r.join("part.md"), "two\n").unwrap();
    let out = cmd(&["check", "doc.md"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "doc.md:1 part file stale warn\n"
    );
    let out = cmd(&["check", "doc.md", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let doc = String::from_utf8_lossy(&out.stdout);
    assert!(
        doc.starts_with("{\"exit\":0,") && doc.contains("\"severity\":\"warn\""),
        "{doc}"
    );
    assert_eq!(cmd(&["run", "doc.md"]).status.code(), Some(1));
    assert_eq!(cmd(&["check", "doc.md"]).status.code(), Some(0));
}
