//! `max-lines=N`: loader text longer than N lines is cut to its first N
//! lines and one note line, before the sink shapes it.

use computed::loader::{LoadError, Loaded};
use computed::marker::{self, Region, Segment};
use computed::render::{self, Action, Loaders, Mode, Rendered, State};

/// Every region loads `text` over a fixed snapshot.
struct Fake(&'static str);

impl Loaders for Fake {
    fn snapshot(&mut self, _: &Region) -> Result<Option<Vec<u8>>, LoadError> {
        Ok(Some(b"snap".to_vec()))
    }
    fn load(&mut self, _: &Region) -> Result<Loaded, LoadError> {
        Ok(Loaded {
            text: self.0.to_string(),
            snapshot: b"snap".to_vec(),
        })
    }
}

fn run(opener: &str, text: &'static str) -> Rendered {
    let file = marker::parse(&format!("{opener}\n<!-- /computed -->\n")).unwrap();
    render::file(&file, Mode::Run { force: false }, true, &mut Fake(text))
}

/// The body the region was rendered with.
fn body(rendered: &Rendered) -> String {
    let Rendered::Written { text, .. } = rendered else {
        panic!("{rendered:?}")
    };
    match &marker::parse(text).unwrap().segments[0] {
        Segment::Region(r) => r.body.clone(),
        Segment::Prose(_) => unreachable!(),
    }
}

#[test]
fn the_opener_takes_a_whole_number_of_at_least_one() {
    let file = marker::parse("<!-- computed tree max-lines=12 -->\n<!-- /computed -->\n").unwrap();
    let Segment::Region(r) = &file.segments[0] else {
        panic!()
    };
    assert_eq!(r.opener.max_lines, Some(12));
    assert!(r.opener.attr("max-lines").is_none());
    assert!(r.opener.canonical().ends_with("max-lines=12 -->"));
    for (value, needle) in [
        ("0", "at least 1"),
        ("ten", "whole number"),
        ("-1", "whole number"),
    ] {
        let e = marker::parse(&format!(
            "<!-- computed tree max-lines={value} -->\n<!-- /computed -->\n"
        ))
        .unwrap_err();
        assert!(e.message.contains(needle), "{value}: {}", e.message);
    }
}

#[test]
fn raw_text_is_cut_with_a_note_as_its_last_line() {
    let r = run(
        "<!-- computed exec cmd=x inputs=x max-lines=2 -->",
        "one\ntwo\nthree\nfour\n\n\n",
    );
    assert_eq!(body(&r), "\none\ntwo\n… 2 more lines\n\n");
    let r = run(
        "<!-- computed exec cmd=x inputs=x max-lines=2 -->",
        "one\ntwo\nthree\n",
    );
    assert_eq!(body(&r), "\none\ntwo\n… 1 more line\n\n");
}

#[test]
fn a_fence_holds_the_note() {
    let r = run(
        "<!-- computed tree max-lines=1 as=fence lang=text -->",
        ".\n├── a\n└── b\n",
    );
    assert_eq!(body(&r), "```text\n.\n… 2 more lines\n```\n");
}

#[test]
fn text_within_the_limit_is_untouched() {
    for text in ["one\ntwo\n", "one\ntwo", ""] {
        let capped = body(&run(
            "<!-- computed exec cmd=x inputs=x max-lines=2 -->",
            text,
        ));
        let plain = body(&run("<!-- computed exec cmd=x inputs=x -->", text));
        assert_eq!(capped, plain, "{text:?}");
    }
}

#[test]
fn a_cut_that_leaves_a_fence_open_is_a_loader_failure() {
    let r = run(
        "<!-- computed exec cmd=x inputs=x max-lines=2 -->",
        "```\ncode\n```\n",
    );
    let Rendered::Unchanged { regions } = &r else {
        panic!("{r:?}")
    };
    assert_eq!(regions[0].action, Some(Action::Failed));
    assert!(
        regions[0].stderr.as_deref().unwrap().contains("fence"),
        "{regions:?}"
    );
}

#[test]
fn the_output_sum_is_over_the_cut_body_so_check_finds_it_fresh() {
    let r = run(
        "<!-- computed exec cmd=x inputs=x max-lines=1 -->",
        "one\ntwo\n",
    );
    let Rendered::Written { text, .. } = &r else {
        panic!()
    };
    let file = marker::parse(text).unwrap();
    let r = render::file(&file, Mode::Check, false, &mut Fake("one\ntwo\n"));
    assert_eq!(r.regions()[0].state, State::Fresh);
}
