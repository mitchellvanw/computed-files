//! `why`: explains a region's state from git history. The closer's `in=`
//! names what the region was rendered from, but only as a sum; the history
//! holds the rest. The latest commit whose template holds the region with
//! the same `in=`, and whose tree recomputes to it, is the render's
//! baseline: its snapshot, taken with the loaders `run` uses, is diffed
//! against the snapshot now. Reading history runs no repository code and
//! needs no trust; the snapshot step runs no command.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::loader::{self, Ctx, Loader, Production};
use crate::marker::{self, Region};
use crate::render::{self, Loaders as _, State};
use crate::report;
use crate::survey::{self, Template};

/// How many commits holding the sum are materialised before giving up.
const TRIES: usize = 8;

/// The repository a template sits in, as git sees it.
struct Repo {
    root: PathBuf,
    /// The template's path from the root, `/`-separated.
    rel: String,
}

/// A commit whose template holds the region with the recorded `in=`.
struct Baseline {
    short: String,
    date: String,
    subject: String,
    full: String,
    /// The region as that commit's template holds it.
    region: Region,
}

/// What a region's explanation came to: its text and exit tier.
struct Answer {
    text: String,
    tier: u8,
}

/// Explains the regions of `file` that `only` or `line` select, all of
/// them when neither does. Exit 0 when every one is answered, fresh ones
/// included; 2 when one could not be: not a repository, no commit holding
/// its `in=`, or none that reproduces it.
pub fn main(
    file: &Path,
    only: Option<&str>,
    line: Option<usize>,
    verbose: bool,
) -> Result<u8, String> {
    let template = match survey::read(file) {
        Ok(Some(t)) => t,
        Ok(None) => return Err(format!("{}: no region", file.display())),
        Err(e) => {
            eprint!("{}", report::error(&e.path, e.line, &e.message));
            return Ok(2);
        }
    };
    let selected: Vec<&Region> = template
        .regions()
        .filter(|r| only.is_none_or(|n| r.opener.name.as_deref() == Some(n)))
        .filter(|r| line.is_none_or(|l| r.line == l))
        .collect();
    if selected.is_empty() {
        return Err(match (only, line) {
            (Some(n), _) => format!("no region is named {n:?}"),
            (_, Some(l)) => format!("no region opens at line {l}"),
            _ => "no region".to_string(),
        });
    }
    let mut repo: Option<Result<Repo, String>> = None;
    let mut loaders = Production::new(template.ctx.clone());
    let mut tier = 0;
    for region in selected {
        let answer = answer(&template, region, &mut loaders, &mut repo, verbose);
        print!("{}", answer.text);
        tier = tier.max(answer.tier);
    }
    Ok(tier)
}

fn answer(
    template: &Template,
    region: &Region,
    loaders: &mut Production,
    repo: &mut Option<Result<Repo, String>>,
    verbose: bool,
) -> Answer {
    let head = |state: &str| {
        let name = region.opener.name.as_deref().unwrap_or("");
        let line = format!(
            "{}:{} {name} {} {state}",
            template.path.display(),
            region.line,
            region.opener.loader
        );
        format!(
            "{}\n",
            line.split_whitespace().collect::<Vec<_>>().join(" ")
        )
    };
    let now = match loaders.snapshot(region) {
        Ok(s) => s,
        Err(loader::LoadError::Hard(m) | loader::LoadError::Failed { stderr: m }) => {
            return Answer {
                text: format!("{}{}", head("error"), indent(&m, 4)),
                tier: 2,
            };
        }
    };
    let state = render::state(region, now.as_deref());
    let mut text = head(&state.to_string());
    let note = |text: &mut String, s: &str| writeln!(text, "    {s}").unwrap();
    match state {
        State::Fresh => return Answer { text, tier: 0 },
        State::Volatile => {
            note(
                &mut text,
                "declares no inputs, so it cannot be known fresh or go stale",
            );
            return Answer { text, tier: 0 };
        }
        State::Unrendered => {
            note(
                &mut text,
                "never rendered: its closer records nothing to compare with",
            );
            return Answer { text, tier: 0 };
        }
        State::Error => unreachable!("a snapshot error returned above"),
        State::Edited if !verbose => {
            note(
                &mut text,
                "the body was changed by hand after it was rendered",
            );
            return Answer { text, tier: 0 };
        }
        State::Edited | State::Stale | State::StaleEdited => {}
    }
    let repo = match repo.get_or_insert_with(|| find_repo(&template.file)) {
        Ok(r) => &*r,
        Err(m) => {
            note(&mut text, m);
            return Answer { text, tier: 2 };
        }
    };
    let recorded = &region.sums.as_ref().expect("a rendered region").input;
    let found = match baseline(repo, region, recorded, state == State::Edited) {
        Ok(found) => found,
        Err(m) => {
            note(&mut text, &m);
            return Answer { text, tier: 2 };
        }
    };
    let mut tier = 0;
    match found {
        Found::Reproduced(base, old) => {
            writeln!(
                text,
                "    rendered from {} {} {}",
                base.short, base.date, base.subject
            )
            .unwrap();
            let paths = differences(
                &mut text,
                repo,
                &base.region,
                region,
                &old,
                now.as_deref(),
                verbose,
            );
            since(&mut text, repo, &base.full, &paths);
            edited(&mut text, state, &base.region, region, verbose);
        }
        Found::Held(base) if state == State::Edited => {
            writeln!(
                text,
                "    rendered in {} {} {}",
                base.short, base.date, base.subject
            )
            .unwrap();
            edited(&mut text, state, &base.region, region, verbose);
        }
        Found::Held(base) => {
            writeln!(
                text,
                "    rendered in {} {} {}\n    that commit's tree did not reproduce in={}: the render read files never committed, or its loader reads them differently now",
                base.short,
                base.date,
                base.subject,
                &recorded[..12]
            )
            .unwrap();
            opener(&mut text, &base.region, region);
            edited(&mut text, state, &base.region, region, verbose);
            tier = 2;
        }
    }
    Answer { text, tier }
}

enum Found {
    /// The commit's tree recomputes to the recorded sum; its snapshot.
    Reproduced(Baseline, Vec<u8>),
    /// The latest commit holding the sum, whose tree does not recompute to it.
    Held(Baseline),
}

fn find_repo(template: &Path) -> Result<Repo, String> {
    let dir = template
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let top = survey::git(dir, &["rev-parse", "--show-toplevel"]).map_err(|_| {
        "not in a git repository: there is no history to explain it from".to_string()
    })?;
    let root = PathBuf::from(top.trim())
        .canonicalize()
        .map_err(|e| format!("repository root: {e}"))?;
    let file = template
        .canonicalize()
        .map_err(|e| format!("{}: {e}", template.display()))?;
    let rel = file
        .strip_prefix(&root)
        .map_err(|_| format!("{} is outside {}", file.display(), root.display()))?;
    let rel = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok(Repo { root, rel })
}

/// The latest commit whose template holds `region` with `in=` equal to
/// `recorded` and whose tree recomputes to it, trying those that hold it
/// newest first. `held_only` stops at the first that holds it, for a region
/// whose inputs are known unchanged.
fn baseline(
    repo: &Repo,
    region: &Region,
    recorded: &str,
    held_only: bool,
) -> Result<Found, String> {
    let git = |args: &[&str]| survey::git(&repo.root, args);
    let pickaxe = format!("-S{recorded}");
    let touched = git(&["log", &pickaxe, "--format=%H", "--", &repo.rel])?;
    let Some(oldest) = touched.lines().last() else {
        return Err(format!(
            "no commit records in={}: the render was never committed",
            &recorded[..12]
        ));
    };
    let log = git(&["log", "--format=%H%x1f%h%x1f%cs%x1f%s", "--", &repo.rel])?;
    let mut latest: Option<Baseline> = None;
    let mut tries = 0;
    for entry in log.lines() {
        let mut f = entry.split('\x1f');
        let (Some(full), Some(short), Some(date), Some(subject)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let Some(old) = held(repo, full, region, recorded) else {
            if full == oldest {
                break;
            }
            continue;
        };
        let base = Baseline {
            short: short.to_string(),
            date: date.to_string(),
            subject: subject.to_string(),
            full: full.to_string(),
            region: old,
        };
        if held_only {
            return Ok(Found::Held(base));
        }
        tries += 1;
        if let Some(snapshot) = reproduce(repo, &base, full, recorded)? {
            return Ok(Found::Reproduced(base, snapshot));
        }
        latest.get_or_insert(base);
        if tries == TRIES || full == oldest {
            break;
        }
    }
    latest
        .map(Found::Held)
        .ok_or_else(|| format!("no commit holds this region with in={}", &recorded[..12]))
}

/// The region as `commit`'s template holds it with `in=` equal to
/// `recorded`: by name, else by the same opener, else any region with it.
fn held(repo: &Repo, commit: &str, region: &Region, recorded: &str) -> Option<Region> {
    let text = survey::git(&repo.root, &["show", &format!("{commit}:{}", repo.rel)]).ok()?;
    let parsed = marker::parse(&text).ok()?;
    let holding: Vec<&Region> = survey::regions(&parsed)
        .filter(|r| r.sums.as_ref().is_some_and(|s| s.input == recorded))
        .collect();
    let canonical = region.opener.canonical();
    holding
        .iter()
        .find(|r| region.opener.name.is_some() && r.opener.name == region.opener.name)
        .or_else(|| holding.iter().find(|r| r.opener.canonical() == canonical))
        .or_else(|| holding.first())
        .map(|r| (*r).clone())
}

/// The snapshot `base.region` takes of `commit`'s tree, when it hashes to
/// `recorded`. The tree is laid down in a temporary directory with an empty
/// `.git`, so ignore rules and the repository bound apply as they did.
fn reproduce(
    repo: &Repo,
    base: &Baseline,
    commit: &str,
    recorded: &str,
) -> Result<Option<Vec<u8>>, String> {
    let dir = materialise(&repo.root, commit)?;
    let path = dir.path().join(&repo.rel);
    let mut loaders = Production::new(Ctx::for_template(&path));
    Ok(match loaders.snapshot(&base.region) {
        Ok(Some(snapshot)) if render::input_sum(&base.region, &snapshot) == recorded => {
            Some(snapshot)
        }
        _ => None,
    })
}

/// `commit`'s tree in a temporary directory: `git archive` into `tar`, so
/// nothing in the repository, its index or its worktrees changes.
fn materialise(root: &Path, commit: &str) -> Result<tempfile::TempDir, String> {
    let dir = tempfile::Builder::new()
        .prefix("computed-why-")
        .tempdir()
        .map_err(|e| format!("temporary directory: {e}"))?;
    let mut archive = Command::new("git")
        .current_dir(root)
        .args(["archive", "--format=tar", commit])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git archive: {e}"))?;
    let tar = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(dir.path())
        .stdin(archive.stdout.take().expect("piped"))
        .output()
        .map_err(|e| format!("tar: {e}"))?;
    let archived = archive
        .wait_with_output()
        .map_err(|e| format!("git archive: {e}"))?;
    if !archived.status.success() || !tar.status.success() {
        return Err(format!(
            "materialising {commit}: {}{}",
            String::from_utf8_lossy(&archived.stderr).trim(),
            String::from_utf8_lossy(&tar.stderr).trim()
        ));
    }
    std::fs::create_dir(dir.path().join(".git")).map_err(|e| format!("{e}"))?;
    Ok(dir)
}

/// Writes what changed between the baseline and now, and returns the
/// changed paths from the repository root, for the commits that changed
/// them.
fn differences(
    text: &mut String,
    repo: &Repo,
    old_region: &Region,
    region: &Region,
    old: &[u8],
    now: Option<&[u8]>,
    verbose: bool,
) -> Vec<String> {
    let mut noted = opener(text, old_region, region);
    let mut paths = Vec::new();
    let from_root = |base: &Path, p: &str| {
        let p = p.split('#').next().unwrap_or(p).trim_end_matches('/');
        survey::normalise(&base.join(p))
            .to_string_lossy()
            .into_owned()
    };
    let template_dir = Path::new(&repo.rel)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let same_loader = old_region.opener.loader == region.opener.loader;
    match now {
        None => {
            writeln!(text, "    now volatile: it declares no inputs").unwrap();
            noted = true;
        }
        // A new loader reads other things: the opener says it all.
        Some(_) if !same_loader => {}
        Some(now) if old == now => {}
        Some(now) if region.opener.loader == "tree" => {
            let src = match Loader::from_opener(&region.opener) {
                Ok(Loader::Tree(args)) => template_dir.join(args.src),
                _ => template_dir.clone(),
            };
            for (sign, path) in listing_diff(old, now) {
                writeln!(text, "    {sign} {path}").unwrap();
                paths.push(from_root(&src, &path));
                noted = true;
            }
        }
        Some(now) => match (loader::entries(old), loader::entries(now)) {
            (Some(old), Some(now)) => {
                for (what, path, before, after) in entries_diff(&old, &now) {
                    writeln!(text, "    {what:7} {path}").unwrap();
                    if verbose && what == "changed" {
                        text.push_str(&content_diff(before, after));
                    }
                    paths.push(from_root(&template_dir, &path));
                    noted = true;
                }
            }
            _ => {
                writeln!(
                    text,
                    "    its snapshot changed: {} bytes, was {}",
                    now.len(),
                    old.len()
                )
                .unwrap();
                noted = true;
            }
        },
    }
    if old_region.indent != region.indent {
        writeln!(
            text,
            "    indentation changed from {:?} to {:?}",
            old_region.indent, region.indent
        )
        .unwrap();
        noted = true;
    }
    if !noted {
        writeln!(
            text,
            "    its opener and inputs are as they were: the loader's format changed since"
        )
        .unwrap();
    }
    paths
}

/// Writes the old and new opener when they differ; whether they did.
fn opener(text: &mut String, old: &Region, region: &Region) -> bool {
    let (was, is) = (old.opener.canonical(), region.opener.canonical());
    if was == is {
        return false;
    }
    writeln!(text, "    opener changed\n        - {was}\n        + {is}").unwrap();
    true
}

/// The hand edit, and under `-v` the diff from the body the baseline holds,
/// when that body is what the closer's `out=` records.
fn edited(text: &mut String, state: State, old: &Region, region: &Region, verbose: bool) {
    if !matches!(state, State::Edited | State::StaleEdited) {
        return;
    }
    writeln!(
        text,
        "    the body was changed by hand after it was rendered"
    )
    .unwrap();
    let out = region.sums.as_ref().map(|s| s.output.as_str());
    if verbose && Some(render::output_sum(&old.body).as_str()) == out {
        text.push_str(&content_diff(old.body.as_bytes(), region.body.as_bytes()));
    }
}

/// The commits since the baseline that changed `paths`, and whether the
/// working tree changes them still.
fn since(text: &mut String, repo: &Repo, commit: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    let range = format!("{commit}..HEAD");
    let mut args = vec!["log", "--format=%h %s", &range, "--"];
    args.extend(paths.iter().map(String::as_str));
    let commits = survey::git(&repo.root, &args).unwrap_or_default();
    if !commits.trim().is_empty() {
        writeln!(text, "    changed since by").unwrap();
        text.push_str(&indent(commits.trim_end(), 8));
    }
    let mut args = vec!["status", "--porcelain", "--"];
    args.extend(paths.iter().map(String::as_str));
    let status = survey::git(&repo.root, &args).unwrap_or_default();
    if !status.trim().is_empty() {
        writeln!(
            text,
            "    not committed: the working tree changes these paths"
        )
        .unwrap();
    }
}

/// The paths a tree listing gained (`+`) and lost (`-`), in byte order.
fn listing_diff(old: &[u8], now: &[u8]) -> Vec<(char, String)> {
    let lines = |b: &[u8]| -> std::collections::BTreeSet<String> {
        String::from_utf8_lossy(b)
            .lines()
            .map(str::to_string)
            .collect()
    };
    let (old, now) = (lines(old), lines(now));
    let mut out: Vec<(char, String)> = now
        .difference(&old)
        .map(|p| ('+', p.clone()))
        .chain(old.difference(&now).map(|p| ('-', p.clone())))
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// The entries added, removed and changed between two entry snapshots, in
/// byte order of path, with the content before and after.
fn entries_diff<'a>(
    old: &[(&'a [u8], &'a [u8])],
    now: &[(&'a [u8], &'a [u8])],
) -> Vec<(&'static str, String, &'a [u8], &'a [u8])> {
    use std::collections::BTreeMap;
    let old: BTreeMap<&[u8], &[u8]> = old.iter().copied().collect();
    let now: BTreeMap<&[u8], &[u8]> = now.iter().copied().collect();
    let mut paths: Vec<&[u8]> = old.keys().chain(now.keys()).copied().collect();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter_map(|p| {
            let name = String::from_utf8_lossy(p).into_owned();
            match (old.get(p), now.get(p)) {
                (None, Some(n)) => Some(("added", name, &b""[..], *n)),
                (Some(o), None) => Some(("removed", name, *o, &b""[..])),
                (Some(o), Some(n)) if o != n => Some(("changed", name, *o, *n)),
                _ => None,
            }
        })
        .collect()
}

/// A unified diff of two contents, indented beneath the line naming them.
fn content_diff(before: &[u8], after: &[u8]) -> String {
    match (std::str::from_utf8(before), std::str::from_utf8(after)) {
        (Ok(b), Ok(a)) => {
            let diff = similar::TextDiff::from_lines(b, a)
                .unified_diff()
                .context_radius(3)
                .to_string();
            indent(diff.trim_end(), 8)
        }
        _ => indent("(not text)", 8),
    }
}

fn indent(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    text.lines().map(|l| format!("{pad}{l}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_decode_what_push_entry_encodes() {
        let snap = b"a.md\x003\x00abc\x00b/c.md\x000\x00\x00";
        assert_eq!(
            loader::entries(snap),
            Some(vec![
                (&b"a.md"[..], &b"abc"[..]),
                (&b"b/c.md"[..], &b""[..])
            ])
        );
        assert_eq!(loader::entries(b""), Some(vec![]));
        assert_eq!(loader::entries(b"a.md\x009\x00abc\x00"), None);
        assert_eq!(loader::entries(b"src/\nsrc/a.rs\n"), None);
    }

    #[test]
    fn a_listing_diff_is_additions_and_removals_in_byte_order() {
        assert_eq!(
            listing_diff(b"a\nb/\nb/c\n", b"a\nb/\nb/d\n"),
            vec![('-', "b/c".to_string()), ('+', "b/d".to_string())]
        );
    }

    #[test]
    fn an_entries_diff_names_what_was_added_removed_and_changed() {
        let old = [(&b"a"[..], &b"1"[..]), (&b"b"[..], &b"2"[..])];
        let now = [(&b"b"[..], &b"3"[..]), (&b"c"[..], &b"4"[..])];
        let diff: Vec<(&str, String)> = entries_diff(&old, &now)
            .into_iter()
            .map(|(w, p, _, _)| (w, p))
            .collect();
        assert_eq!(
            diff,
            vec![
                ("removed", "a".to_string()),
                ("changed", "b".to_string()),
                ("added", "c".to_string())
            ]
        );
    }
}
