//! `adopt`: a hand edit inside a `file` region written back into the file
//! it copies, and the region rendered again so it is fresh. The reverse of
//! what `run` does, and only where it has one: the sink's own lines and the
//! opener's indentation come off, what is left replaces the part of the
//! source the loader read, and rendering the new source must give back the
//! edited body exactly, or nothing is written.
//!
//! Refusals are tier 1, like `run`'s: a loader whose body is computed, a
//! source that changed since the render (the edit and that change would
//! meet here, and adopt does not merge), an edit that does not round-trip.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::loader::{Loader, Production};
use crate::marker::{self, Region, Sink};
use crate::render::{self, Loaders as _, Mode, Rendered, State};
use crate::survey::{self, Template};
use crate::{fs, report, sink};

/// What adopting one region would write into its source.
struct Adoption {
    /// The source file.
    path: PathBuf,
    /// The source as the invocation would spell it.
    shown: String,
    old: String,
    new: String,
}

/// Adopts every edited region of `file` that `only` selects, all of them
/// when it is empty. Exit 0 when each was adopted or had nothing to adopt,
/// 1 when one was refused or `--dry-run` would write, 2 when the tool could
/// not answer.
pub fn main(file: &Path, only: &[String], dry_run: bool, verbose: bool) -> Result<u8, String> {
    let template = match survey::read(file) {
        Ok(Some(t)) => t,
        Ok(None) => return Err(format!("{}: no region", file.display())),
        Err(e) => {
            eprint!("{}", report::error(&e.path, e.line, &e.message));
            return Ok(2);
        }
    };
    if let Some(name) = only.iter().find(|n| {
        !template
            .regions()
            .any(|r| r.opener.name.as_ref() == Some(n))
    }) {
        return Err(format!("no region is named {name:?}"));
    }
    let selected: Vec<&Region> = template
        .regions()
        .filter(|r| only.is_empty() || r.opener.name.as_ref().is_some_and(|n| only.contains(n)))
        .collect();
    let mut loaders = Production::new(template.ctx.clone());
    let mut tier = 0;
    let mut lines = Vec::new();
    let mut adoptions: Vec<(usize, Adoption)> = Vec::new();
    let mut sources = BTreeSet::new();
    for region in selected {
        let say = |state: &str, action: &str| {
            let name = region.opener.name.as_deref().unwrap_or("");
            let line = format!(
                "{}:{} {name} {} {state} {action}",
                template.path.display(),
                region.line,
                region.opener.loader
            );
            line.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        let state = match loaders.snapshot(region) {
            Ok(snapshot) => render::state(region, snapshot.as_deref()),
            Err(
                crate::loader::LoadError::Hard(m) | crate::loader::LoadError::Failed { stderr: m },
            ) => {
                lines.push(format!("{}\n    {m}", say("error", "skipped")));
                tier = 2;
                continue;
            }
        };
        let adoption = match state {
            State::Edited => adoption(&template, region),
            State::StaleEdited => Err(
                "the source changed since the render, or the opener did, and adopting would overwrite that change: merge the edit by hand, or `run --force` to drop it"
                    .to_string(),
            ),
            _ => {
                if verbose {
                    lines.push(say(&state.to_string(), "nothing to adopt"));
                }
                continue;
            }
        };
        let state = state.to_string();
        match adoption {
            Ok(a) if !sources.insert(a.path.clone()) => {
                lines.push(say(
                    &state,
                    &format!("refused; another region here adopts into {}", a.shown),
                ));
                tier = tier.max(1);
            }
            Ok(a) => {
                let verb = if dry_run {
                    "would adopt into"
                } else {
                    "adopted into"
                };
                lines.push(say(&state, &format!("{verb} {}", a.shown)));
                adoptions.push((region.line, a));
            }
            Err(reason) => {
                lines.push(say(&state, &format!("refused; {reason}")));
                tier = tier.max(1);
            }
        }
    }
    if dry_run {
        for (_, a) in &adoptions {
            print!(
                "{}",
                report::diff(Path::new(&a.shown), &a.old, &a.new, None)
            );
        }
        if !adoptions.is_empty() {
            tier = tier.max(1);
        }
    } else if !adoptions.is_empty() {
        tier = tier.max(write(&template, &adoptions, &mut lines));
    }
    for line in lines {
        eprintln!("{line}");
    }
    Ok(tier)
}

/// Writes each source, then renders the adopted regions from them and
/// writes the template. A region that does not come back as it was edited
/// puts its source back and refuses: the check before writing reads the
/// source as the sink does, this one as the loader does.
fn write(template: &Template, adoptions: &[(usize, Adoption)], lines: &mut Vec<String>) -> u8 {
    for (_, a) in adoptions {
        if let Err(e) = fs::replace(&a.path, &a.old, &a.new) {
            lines.push(report::error(Path::new(&a.shown), None, &e.to_string()));
            return 2;
        }
    }
    let adopted: Vec<usize> = adoptions.iter().map(|(l, _)| *l).collect();
    let select = |r: &Region| adopted.contains(&r.line);
    let mut loaders = Production::new(template.ctx.clone());
    let force = Mode::Run { force: true };
    let rendered = render::file_where(&template.parsed, force, false, &select, &mut loaders);
    let text = match rendered {
        Rendered::Written { text, .. } => text,
        Rendered::Unchanged { .. } => return 0,
        Rendered::Refused { .. } => unreachable!("force renders over a hand edit"),
        Rendered::Error { line, message } => {
            restore(adoptions);
            lines.push(report::error(&template.path, Some(line), &message));
            return 2;
        }
    };
    let same = marker::parse(&text).is_ok_and(|parsed| {
        survey::regions(&parsed)
            .zip(template.regions())
            .filter(|(_, was)| adopted.contains(&was.line))
            .all(|(now, was)| now.body == was.body)
    });
    if !same {
        restore(adoptions);
        lines.push(format!(
            "{}: the adopted sources do not render back to the edited bodies; put back, nothing written",
            template.path.display()
        ));
        return 1;
    }
    match fs::replace(&template.file, &template.text, &text) {
        Ok(_) => 0,
        Err(e) => {
            lines.push(report::error(&template.path, None, &e.to_string()));
            2
        }
    }
}

fn restore(adoptions: &[(usize, Adoption)]) {
    for (_, a) in adoptions {
        let _ = fs::replace(&a.path, &a.new, &a.old);
    }
}

/// What adopting `region`'s body would write into its source, or why it
/// cannot be.
fn adoption(template: &Template, region: &Region) -> Result<Adoption, String> {
    let args = match Loader::from_opener(&region.opener) {
        Ok(Loader::File(args)) => args,
        Ok(_) => {
            return Err(format!(
                "a {} body is computed, not copied from a file: change what it reads instead",
                region.opener.loader
            ));
        }
        Err(crate::loader::LoadError::Hard(m) | crate::loader::LoadError::Failed { stderr: m }) => {
            return Err(m);
        }
    };
    let path = template.ctx.region_root.join(&args.src);
    let shown = survey::normalise(&path).display().to_string();
    let old = std::fs::read_to_string(&path).map_err(|e| format!("{shown}: {e}"))?;
    if marker::strip_sums(old.as_bytes()).as_ref() != old.as_bytes() {
        return Err(format!(
            "{shown} holds rendered regions of its own, and adopt does not write into them"
        ));
    }
    let text = unshape(region)?;
    let new = splice(&old, read_range(&old), &text);
    let back = sink::body(
        region.opener.sink,
        &region.opener.lang,
        region.opener.max_lines,
        &marker::strip_sums(new.as_bytes()),
    )
    .map_err(|m| {
        format!("the edit does not round-trip: {shown} written back would fail to render: {m}")
    })?;
    if render::shape(region, &back) != region.body {
        return Err(format!(
            "the edit does not round-trip: {shown} written back would render a different body; keep the sink's own lines and the region's indentation"
        ));
    }
    Ok(Adoption {
        path,
        shown,
        old,
        new,
    })
}

/// The bytes of `source` the loader's text came from: all of it but the
/// trailing newlines normalisation strips. Where a slice of the file is
/// read, this is the slice.
fn read_range(source: &str) -> Range<usize> {
    0..source.trim_end_matches(['\n', '\r']).len()
}

/// `source` with `range` replaced by `text`, an LF text written in the
/// source's own line endings.
fn splice(source: &str, range: Range<usize>, text: &str) -> String {
    let text = if source.contains("\r\n") {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    };
    format!("{}{text}{}", &source[..range.start], &source[range.end..])
}

/// The loader text a body was shaped from: the opener's indentation and
/// line endings taken off every line, then the sink's own lines.
fn unshape(region: &Region) -> Result<String, String> {
    let mut lf = String::new();
    for line in region.body.split_inclusive('\n') {
        let text = line.strip_suffix('\n').unwrap_or(line);
        let text = text.strip_suffix('\r').unwrap_or(text);
        if !text.is_empty() {
            let text = text
                .strip_prefix(region.indent.as_str())
                .ok_or("a body line lost the region's indentation")?;
            lf.push_str(text);
        }
        lf.push('\n');
    }
    let sink = region.opener.sink;
    if sink == Sink::Raw {
        // A blank line, the text, a blank line.
        let text = lf.strip_prefix('\n').unwrap_or(&lf);
        let text = text
            .strip_suffix("\n\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text);
        Ok(text.to_string())
    } else if sink == Sink::Fence {
        // The opening fence, the text, the closing fence.
        let lines: Vec<&str> = lf.lines().collect();
        match lines.as_slice() {
            [_, inner @ .., _] => Ok(inner.join("\n")),
            _ => Err("the fence lines are gone".to_string()),
        }
    } else {
        Err("this sink's body cannot be turned back into the file".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(text: &str) -> Region {
        match marker::parse(text).unwrap().segments.remove(0) {
            marker::Segment::Region(r) => r,
            marker::Segment::Prose(_) => unreachable!(),
        }
    }

    #[test]
    fn unshape_takes_off_what_each_sink_and_the_indent_put_on() {
        let raw = region("<!-- computed file src=a -->\n\na\n\nb\n\n<!-- /computed -->\n");
        assert_eq!(unshape(&raw).unwrap(), "a\n\nb");
        let empty = region("<!-- computed file src=a -->\n\n\n<!-- /computed -->\n");
        assert_eq!(unshape(&empty).unwrap(), "");
        let fence = region(
            "  <!-- computed file src=a as=fence -->\r\n  ```\r\n  a\r\n\r\n  b\r\n  ```\r\n  <!-- /computed -->\r\n",
        );
        assert_eq!(unshape(&fence).unwrap(), "a\n\nb");
        let lost = region("  <!-- computed file src=a -->\n\nx\n\n  <!-- /computed -->\n");
        assert!(unshape(&lost).is_err());
    }

    #[test]
    fn splice_keeps_the_trailing_newlines_and_the_line_endings() {
        let s = "a\r\nb\r\n\r\n";
        assert_eq!(splice(s, read_range(s), "x\ny"), "x\r\ny\r\n\r\n");
        let s = "a";
        assert_eq!(splice(s, read_range(s), "b"), "b");
    }
}
