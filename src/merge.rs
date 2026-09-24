//! `merge`: a git merge driver for templates. Two branches that each
//! re-render a region write different bodies and different sums into it,
//! and a line merge makes that a conflict nobody should resolve by hand:
//! the right body is whatever the merged inputs render to.
//!
//! So the merge is by structure. Each version's region bodies and closers
//! are set aside and a placeholder line stands in for them; `git merge-file`
//! merges what is left, prose and openers, as it would any text, and its
//! conflicts stay conflicts. Each placeholder then takes the region's body
//! and closer from the side that changed them, or, when both did, ours with
//! both sums stripped: the region is unrendered, and the next `run`, the
//! pre-commit hook, renders it from the merged inputs.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::marker;
use crate::survey;

/// Starts every placeholder line; followed by the region's key in hex.
const PLACEHOLDER: &str = "@@computed-merge-region@@ ";

/// A region's body and closer, as one version holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tail {
    body: String,
    closer: String,
    indent: String,
}

impl Tail {
    fn text(&self) -> String {
        format!("{}{}", self.body, self.closer)
    }

    /// The body with a closer that carries no sums.
    fn unrendered(&self) -> String {
        let term = &self.closer[self.closer.trim_end_matches(['\n', '\r']).len()..];
        format!(
            "{}{}{}{term}",
            self.body,
            self.indent,
            marker::rendered_closer(None)
        )
    }
}

/// One version with its region tails replaced by placeholder lines.
struct Skeleton {
    text: String,
    tails: BTreeMap<String, Tail>,
}

/// The key a region is matched by across versions: its name, else its
/// canonical opener, with the ordinal of a repeat.
fn skeleton(text: &str) -> Option<Skeleton> {
    let parsed = marker::parse(text).ok()?;
    let mut out = String::new();
    let mut tails = BTreeMap::new();
    for segment in &parsed.segments {
        match segment {
            marker::Segment::Prose(p) => out.push_str(p),
            marker::Segment::Region(r) => {
                let base = r
                    .opener
                    .name
                    .clone()
                    .unwrap_or_else(|| r.opener.canonical());
                let key = (0..)
                    .map(|n| {
                        if n == 0 {
                            base.clone()
                        } else {
                            format!("{base}#{n}")
                        }
                    })
                    .find(|k| !tails.contains_key(k))
                    .expect("an unused key");
                out.push_str(&r.raw_opener);
                out.push_str(PLACEHOLDER);
                out.push_str(&hex(&key));
                out.push('\n');
                tails.insert(
                    key,
                    Tail {
                        body: r.body.clone(),
                        closer: r.raw_closer.clone(),
                        indent: r.indent.clone(),
                    },
                );
            }
        }
    }
    Some(Skeleton { text: out, tails })
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<String> {
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// A region's body and closer after the merge: the side that changed them,
/// or ours unrendered when both did, differently.
fn resolve(base: Option<&Tail>, ours: Option<&Tail>, theirs: Option<&Tail>) -> String {
    match (ours, theirs) {
        (Some(o), Some(t)) if o == t => o.text(),
        (Some(o), Some(t)) if base == Some(o) => t.text(),
        (Some(o), Some(t)) if base == Some(t) => o.text(),
        (Some(o), Some(_)) => o.unrendered(),
        (Some(o), None) => o.text(),
        (None, Some(t)) => t.text(),
        (None, None) => base.map(Tail::text).unwrap_or_default(),
    }
}

/// The merged text and the number of conflicts left in it.
pub struct Merged {
    pub text: Vec<u8>,
    pub conflicts: usize,
}

/// Merges `ours` and `theirs` from `base`. Three versions that parse are
/// merged by structure; anything else is merged as `git merge-file` merges
/// text.
pub fn merge(base: &[u8], ours: &[u8], theirs: &[u8]) -> Result<Merged, String> {
    let parse = |b: &[u8]| {
        std::str::from_utf8(b)
            .ok()
            .filter(|t| !t.contains(PLACEHOLDER))
            .and_then(skeleton)
    };
    let (Some(b), Some(o), Some(t)) = (parse(base), parse(ours), parse(theirs)) else {
        return merge_file(base, ours, theirs);
    };
    let merged = merge_file(b.text.as_bytes(), o.text.as_bytes(), t.text.as_bytes())?;
    let text = String::from_utf8(merged.text)
        .map_err(|_| "git merge-file: output is not UTF-8".to_string())?;
    let keys: BTreeSet<&String> = b
        .tails
        .keys()
        .chain(o.tails.keys())
        .chain(t.tails.keys())
        .collect();
    let resolved: BTreeMap<&String, String> = keys
        .into_iter()
        .map(|k| (k, resolve(b.tails.get(k), o.tails.get(k), t.tails.get(k))))
        .collect();
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let key = line
            .trim_end_matches(['\n', '\r'])
            .strip_prefix(PLACEHOLDER)
            .and_then(unhex);
        match key.as_ref().and_then(|k| resolved.get(k)) {
            Some(tail) => out.push_str(tail),
            None => out.push_str(line),
        }
    }
    Ok(Merged {
        text: out.into_bytes(),
        conflicts: merged.conflicts,
    })
}

/// `git merge-file -p` over three versions written to a temporary directory.
fn merge_file(base: &[u8], ours: &[u8], theirs: &[u8]) -> Result<Merged, String> {
    let dir = tempfile::tempdir().map_err(|e| format!("temporary directory: {e}"))?;
    let write = |name: &str, bytes: &[u8]| -> Result<PathBuf, String> {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    };
    let (b, o, t) = (
        write("base", base)?,
        write("ours", ours)?,
        write("theirs", theirs)?,
    );
    let out = Command::new("git")
        .args([
            "merge-file",
            "-p",
            "-L",
            "ours",
            "-L",
            "base",
            "-L",
            "theirs",
        ])
        .args([&o, &b, &t])
        .output()
        .map_err(|e| format!("git merge-file: {e}"))?;
    // The exit status is the number of conflicts, or negative on error.
    match out.status.code() {
        Some(n @ 0..=127) => Ok(Merged {
            text: out.stdout,
            conflicts: n as usize,
        }),
        _ => Err(format!(
            "git merge-file: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// The driver git runs as `computed merge %O %A %B %P`: merges into `ours`,
/// exit 0 when no conflict is left, 1 when one is, as git expects.
pub fn driver(base: &Path, ours: &Path, theirs: &Path) -> Result<u8, String> {
    let read = |p: &Path| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    let merged = merge(&read(base)?, &read(ours)?, &read(theirs)?)?;
    std::fs::write(ours, &merged.text).map_err(|e| format!("{}: {e}", ours.display()))?;
    Ok(u8::from(merged.conflicts > 0))
}

/// The attribute lines that route Markdown through the driver.
const ATTRIBUTES: &[&str] = &["*.md merge=computed", "*.markdown merge=computed"];

/// The driver's local configuration: per clone, like trust, since a
/// repository cannot name a program for git to run.
const CONFIG: &[(&str, &str)] = &[
    ("merge.computed.name", "computed"),
    ("merge.computed.driver", "computed merge %O %A %B %P"),
];

/// Adds the attribute lines to the repository's `.gitattributes` where
/// missing and sets the driver in the clone's own configuration, printing
/// what it did.
pub fn install() -> Result<u8, String> {
    let here = Path::new(".");
    let top = survey::git(here, &["rev-parse", "--show-toplevel"])
        .map_err(|_| "not in a git repository".to_string())?;
    let root = PathBuf::from(top.trim());
    let path = root.join(".gitattributes");
    let current = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let present: BTreeSet<&str> = current.lines().map(str::trim).collect();
    let missing: Vec<&str> = ATTRIBUTES
        .iter()
        .copied()
        .filter(|a| !present.contains(a))
        .collect();
    let mut done = Vec::new();
    if !missing.is_empty() {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut text = String::new();
        if !current.is_empty() && !current.ends_with('\n') {
            text.push('\n');
        }
        for a in &missing {
            text.push_str(a);
            text.push('\n');
            done.push(format!("added `{a}` to .gitattributes"));
        }
        file.write_all(text.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    for (key, value) in CONFIG {
        let set = survey::git(&root, &["config", "--local", "--get", key]).unwrap_or_default();
        if set.trim_end_matches('\n') != *value {
            survey::git(&root, &["config", "--local", key, value])?;
            done.push(format!("set {key} = {value}"));
        }
    }
    if done.is_empty() {
        println!("already installed");
    }
    for line in done {
        println!("{line}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(body: &str) -> Tail {
        Tail {
            body: body.to_string(),
            closer: "  <!-- /computed in=a out=b -->\n".to_string(),
            indent: "  ".to_string(),
        }
    }

    #[test]
    fn a_region_takes_the_side_that_changed_it_or_ours_unrendered() {
        let (b, o, t) = (tail("b\n"), tail("o\n"), tail("t\n"));
        assert_eq!(resolve(Some(&b), Some(&b), Some(&t)), t.text());
        assert_eq!(resolve(Some(&b), Some(&o), Some(&b)), o.text());
        assert_eq!(resolve(Some(&b), Some(&o), Some(&o)), o.text());
        assert_eq!(
            resolve(Some(&b), Some(&o), Some(&t)),
            "o\n  <!-- /computed -->\n"
        );
        assert_eq!(resolve(None, None, Some(&t)), t.text());
    }

    #[test]
    fn keys_are_names_else_openers_with_the_ordinal_of_a_repeat() {
        let s = skeleton(
            "<!-- computed tree -->\n<!-- /computed -->\n<!-- computed tree -->\n<!-- /computed -->\n<!-- computed tree name=x -->\n<!-- /computed -->\n",
        )
        .unwrap();
        let keys: Vec<&String> = s.tails.keys().collect();
        assert_eq!(
            keys,
            ["<!-- computed tree -->", "<!-- computed tree -->#1", "x"]
        );
        assert_eq!(unhex(&hex("x#1")), Some("x#1".to_string()));
    }
}
