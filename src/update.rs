//! `computed update`: fetches every `remote` region's url and writes the
//! SHA-256 of what came back into the opener's `sha256=`, the rest of the
//! line as it was. It renders nothing: the moved pin changes the opener,
//! which makes the region stale, and the next `run` fetches and renders it.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::allow::Allowed;
use crate::marker::{self, Region, Segment};
use crate::remote::{self, RemoteArgs};
use crate::render::{Action, Mode, RegionReport, State};
use crate::{fs, report};

/// What updating one file came to.
#[derive(Default)]
struct Outcome {
    tier: u8,
    error: Option<(Option<usize>, String)>,
    regions: Vec<RegionReport>,
    diff: Option<String>,
    names: Vec<String>,
}

/// Updates the pins in `files` and returns the exit tier: 1 when a pin was
/// written, or would be under `dry_run`, or a url is not allowed; 2 when a
/// url could not be fetched, a file could not be read or parsed, or a name
/// in `only` names nothing.
pub fn run(
    files: &[PathBuf],
    dry_run: bool,
    only: &[String],
    allowed: &Allowed,
    verbose: bool,
    json: bool,
) -> u8 {
    let mode = if dry_run {
        Mode::DryRun { force: false }
    } else {
        Mode::Run { force: false }
    };
    let mut tier = 0;
    let mut names = Vec::new();
    let mut outcomes = Vec::new();
    for path in files {
        let outcome = file(path, dry_run, only, allowed);
        tier = tier.max(outcome.tier);
        names.extend(outcome.names.iter().cloned());
        if !json {
            let mut err = std::io::stderr().lock();
            if let Some((line, message)) = &outcome.error {
                let _ = err.write_all(report::error(path, *line, message).as_bytes());
            }
            for block in report::regions(path, &outcome.regions, mode, verbose) {
                let _ = err.write_all(block.as_bytes());
            }
            if let Some(diff) = &outcome.diff {
                print!("{diff}");
            }
        }
        outcomes.push((path, outcome));
    }
    let mut unknown = Vec::new();
    for name in only.iter().filter(|n| !names.contains(n)) {
        let message = format!("no region is named {name:?}");
        if !json {
            eprintln!("computed: {message}");
        }
        unknown.push(message);
        tier = 2;
    }
    if json {
        let dot = PathBuf::from(".");
        let mut files: Vec<report::FileJson<'_>> = outcomes
            .iter()
            .filter(|(_, o)| o.error.is_some() || !o.regions.is_empty())
            .map(|(p, o)| report::FileJson {
                path: p,
                error: o.error.as_ref().map(|(l, m)| (*l, m.as_str())),
                regions: &o.regions,
                diff: o.diff.as_deref(),
            })
            .collect();
        files.extend(unknown.iter().map(|m| report::FileJson {
            path: &dot,
            error: Some((None, m.as_str())),
            regions: &[],
            diff: None,
        }));
        print!("{}", report::json(&files, tier));
    }
    tier
}

fn file(path: &Path, dry_run: bool, only: &[String], allowed: &Allowed) -> Outcome {
    let error = |line, message: String| Outcome {
        tier: 2,
        error: Some((line, message)),
        ..Outcome::default()
    };
    let text = match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(e) if marker::has_marker(&String::from_utf8_lossy(e.as_bytes())) => {
                return error(None, "not UTF-8".to_string());
            }
            Err(_) => return Outcome::default(),
        },
        Err(e) => return error(None, format!("unreadable: {e}")),
    };
    if !text.contains("<!--") {
        return Outcome::default();
    }
    let mut parsed = match marker::parse(&text) {
        Ok(p) => p,
        Err(e) => return error(Some(e.line), e.message),
    };
    let mut outcome = Outcome::default();
    for segment in &mut parsed.segments {
        let Segment::Region(region) = segment else {
            continue;
        };
        outcome.names.extend(region.opener.name.clone());
        let selected = only.is_empty()
            || region
                .opener
                .name
                .as_ref()
                .is_some_and(|n| only.contains(n));
        if region.opener.loader != "remote" || !selected {
            continue;
        }
        let report = pin_region(region, dry_run, allowed);
        outcome.tier = outcome.tier.max(match report.action {
            Some(Action::Error) => 2,
            Some(Action::Written | Action::WouldWrite | Action::Disallowed) => 1,
            _ => 0,
        });
        outcome.regions.push(report);
    }
    let new = marker::serialise(&parsed);
    if new == text {
        return outcome;
    }
    if dry_run {
        outcome.diff = Some(report::diff(path, &text, &new, None));
    } else if let Err(e) = fs::replace(path, &text, &new) {
        return error(None, format!("writing: {e}"));
    }
    outcome
}

/// Fetches the region's url and moves its pin to what came back. A url the
/// allowlist does not cover is skipped, reported as `run` reports it, with
/// the state a missing pin is known to be and a present one assumed.
fn pin_region(region: &mut Region, dry_run: bool, allowed: &Allowed) -> RegionReport {
    let report = |state, action, message: Option<String>| RegionReport {
        line: region.line,
        name: region.opener.name.clone(),
        loader: region.opener.loader.clone(),
        state,
        action: Some(action),
        stderr: message,
    };
    let args = match RemoteArgs::from_opener(&region.opener) {
        Ok(args) => args,
        Err(e) => return report(State::Error, Action::Error, Some(format!("{e:?}"))),
    };
    if !allowed.allows(&args.url) {
        let state = match args.sha256 {
            None => State::Stale,
            Some(_) => State::Fresh,
        };
        return report(
            state,
            Action::Disallowed,
            Some(remote::not_allowed(&args.url)),
        );
    }
    let body = match remote::fetch(&args.url, args.timeout, allowed) {
        Ok(body) => body,
        Err(e) => return report(State::Error, Action::Error, Some(e)),
    };
    let got = remote::digest(&body);
    if args.sha256.as_deref() == Some(got.as_str()) {
        return report(State::Fresh, Action::Fresh, None);
    }
    let Some(line) = repin(&region.raw_opener, &got) else {
        return report(
            State::Error,
            Action::Error,
            Some("could not find where to write sha256= in the opener".to_string()),
        );
    };
    let message = match &args.sha256 {
        Some(old) => format!("was {old}\nnow {got}"),
        None => format!("now {got}"),
    };
    region.raw_opener = line;
    let action = if dry_run {
        Action::WouldWrite
    } else {
        Action::Written
    };
    report(State::Stale, action, Some(message))
}

/// The opener line with its `sha256=` value replaced by `pin`, or, with
/// none, `sha256=pin` added after the `url=` value. Everything else on the
/// line stays as written. `None` when the result would not parse back to
/// the same opener with the new pin.
fn repin(raw: &str, pin: &str) -> Option<String> {
    let value_end = |from: usize| -> usize {
        let rest = &raw[from..];
        if let Some(quoted) = rest.strip_prefix('"') {
            let mut escaped = false;
            for (i, c) in quoted.char_indices() {
                match c {
                    '\\' if !escaped => escaped = true,
                    '"' if !escaped => return from + 1 + i + 1,
                    _ => escaped = false,
                }
            }
            raw.len()
        } else {
            from + rest.find([' ', '\t', '\r', '\n']).unwrap_or(rest.len())
        }
    };
    let key = |k: &str| {
        [" ", "\t"]
            .iter()
            .filter_map(|ws| raw.find(&format!("{ws}{k}=")))
            .min()
            .map(|at| at + 1 + k.len() + 1)
    };
    let line = match key("sha256") {
        Some(start) => format!("{}{pin}{}", &raw[..start], &raw[value_end(start)..]),
        None => {
            let end = value_end(key("url")?);
            format!("{} sha256={pin}{}", &raw[..end], &raw[end..])
        }
    };
    let region = |opener: &str| {
        let eol = if opener.ends_with('\n') { "" } else { "\n" };
        marker::parse(&format!("{opener}{eol}<!-- /computed -->\n")).ok()
    };
    let (before, after) = (region(raw)?, region(&line)?);
    let opener = |f: &marker::File| match f.segments.first() {
        Some(Segment::Region(r)) => Some(r.opener.clone()),
        _ => None,
    };
    let (before, after) = (opener(&before)?, opener(&after)?);
    let others = |o: &marker::Opener| -> Vec<(String, String)> {
        o.attrs
            .iter()
            .filter(|(k, _)| k != "sha256")
            .cloned()
            .collect()
    };
    (after.attr("sha256") == Some(pin)
        && others(&before) == others(&after)
        && before.name == after.name)
        .then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const B: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn repin_replaces_or_adds_the_pin_and_keeps_the_rest_of_the_line() {
        assert_eq!(
            repin(
                &format!("  <!--  computed remote  url=https://a/x?sha256=q  sha256={A} name=n | do not edit; run computed -->\r\n"),
                B
            )
            .unwrap(),
            format!("  <!--  computed remote  url=https://a/x?sha256=q  sha256={B} name=n | do not edit; run computed -->\r\n")
        );
        assert_eq!(
            repin(
                &format!("<!-- computed remote sha256=\"{A}\" url=https://a -->\n"),
                B
            )
            .unwrap(),
            format!("<!-- computed remote sha256={B} url=https://a -->\n")
        );
        assert_eq!(
            repin("<!-- computed remote url=\"https://a/b c\" name=n -->\n", B).unwrap(),
            format!("<!-- computed remote url=\"https://a/b c\" sha256={B} name=n -->\n")
        );
        assert_eq!(
            repin("<!-- computed remote url=https://a -->", B).unwrap(),
            format!("<!-- computed remote url=https://a sha256={B} -->")
        );
    }
}
