//! `watch`: `run` again whenever something a region could read changes,
//! until Ctrl-C.
//!
//! Every marker path resolves inside its template's repository root, so the
//! repository roots of the templates, with the directories the invocation
//! named, bound everything a region can read. They are watched recursively.
//! A change there triggers a pass unless the `.gitignore` rules ignore it,
//! as the walk would; a change to a file a snapshot read, ignored or not,
//! always does. What a pass writes comes back as events too: an event for a
//! file that still holds the bytes the pass wrote is the tool's own write
//! and is dropped. What to watch is re-derived after every pass, so a new
//! template, or a region reading a new file, is picked up.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecursiveMode, Watcher};
use sha2::{Digest, Sha256};

use crate::fs::{self, Ignores};

/// What one `run` came to, as `watch` needs it.
pub struct Pass {
    /// The templates discovered.
    pub files: Vec<PathBuf>,
    /// The canonical files their snapshots read.
    pub read: BTreeSet<PathBuf>,
    /// The canonical files the pass wrote.
    pub written: BTreeSet<PathBuf>,
}

/// How long the tree must stay quiet before a batch of events runs a pass.
const QUIET: Duration = Duration::from_millis(200);
/// A pass runs at the latest this long after a batch began, however busy
/// the tree stays.
const PATIENCE: Duration = Duration::from_secs(2);

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// What the watcher knows between passes.
#[derive(Default)]
struct State {
    /// Canonical directories watched recursively.
    roots: BTreeSet<PathBuf>,
    /// Canonical files named on the command line.
    named: BTreeSet<PathBuf>,
    templates: BTreeSet<PathBuf>,
    read: BTreeSet<PathBuf>,
    /// The sum of what the tool last wrote to each file.
    own: HashMap<PathBuf, [u8; 32]>,
}

fn sum(path: &Path) -> Option<[u8; 32]> {
    std::fs::read(path).ok().map(|b| Sha256::digest(b).into())
}

/// Whether the `.gitignore` rules of `path`'s repository ignore it or a
/// directory above it, as a walk from the repository root would. `.git`
/// always counts as ignored.
fn ignored(path: &Path) -> bool {
    if path.components().any(|c| c.as_os_str() == ".git") {
        return true;
    }
    let Some(repo) = path.parent().and_then(fs::repo_root) else {
        return false;
    };
    let Ok(rel) = path.strip_prefix(&repo) else {
        return false;
    };
    let count = rel.components().count();
    let mut dir = repo.clone();
    let mut rules = Ignores::at(Some(&repo), &repo);
    for (i, c) in rel.components().enumerate() {
        let child = dir.join(c);
        let is_dir = i + 1 < count || child.is_dir();
        if rules.ignores(&child, is_dir) {
            return true;
        }
        if i + 1 < count {
            rules = rules.enter(&child);
        }
        dir = child;
    }
    false
}

impl State {
    /// Whether a changed path calls for a pass.
    fn relevant(&mut self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with(".computed-") && name.ends_with(".tmp") {
            return false;
        }
        if let Some(written) = self.own.get(path) {
            if sum(path).as_ref() == Some(written) {
                return false;
            }
            self.own.remove(path);
        }
        if self.read.contains(path) || self.templates.contains(path) || self.named.contains(path) {
            return true;
        }
        self.roots.iter().any(|r| path.starts_with(r)) && !ignored(path)
    }

    /// Takes in a pass: its own writes, its templates and what they read,
    /// and the repository roots to watch that were not watched yet.
    fn after(&mut self, pass: Pass, watcher: &mut dyn Watcher) {
        for w in pass.written {
            if let Some(s) = sum(&w) {
                self.own.insert(w, s);
            }
        }
        self.templates = pass
            .files
            .iter()
            .filter_map(|f| f.canonicalize().ok())
            .collect();
        self.read = pass.read;
        let wanted: BTreeSet<PathBuf> = self
            .templates
            .iter()
            .chain(&self.read)
            .filter_map(|f| {
                let dir = f.parent()?;
                Some(fs::repo_root(dir).unwrap_or_else(|| dir.to_path_buf()))
            })
            .collect();
        for dir in wanted {
            if self.roots.iter().any(|r| dir.starts_with(r)) {
                continue;
            }
            match watcher.watch(&dir, RecursiveMode::Recursive) {
                Ok(()) => {
                    self.roots.insert(dir);
                }
                Err(e) => eprintln!("computed: watching {}: {e}", dir.display()),
            }
        }
    }
}

fn paths(event: notify::Result<Event>) -> Vec<PathBuf> {
    match event {
        Ok(e) if matches!(e.kind, EventKind::Access(_)) => Vec::new(),
        Ok(e) => e.paths,
        // A lost event is a change we cannot place: look at everything.
        Err(_) => vec![PathBuf::new()],
    }
}

/// Watches `targets` (the current directory when empty) and calls `pass`,
/// which runs `run` and prints its report, once at the start and again
/// after every batch of relevant changes. The first pass failing is the
/// command failing; a later one is reported and watching goes on. Returns
/// 0 on Ctrl-C.
pub fn watch(
    targets: &[PathBuf],
    announce: bool,
    mut pass: impl FnMut() -> Result<Pass, String>,
) -> Result<u8, String> {
    let targets = if targets.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        targets.to_vec()
    };
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
    }
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| format!("watching: {e}"))?;
    let mut state = State::default();
    for t in &targets {
        let canon = t
            .canonicalize()
            .map_err(|e| format!("{}: {e}", t.display()))?;
        if canon.is_dir() {
            watcher
                .watch(&canon, RecursiveMode::Recursive)
                .map_err(|e| format!("watching {}: {e}", t.display()))?;
            state.roots.insert(canon);
        } else {
            state.named.insert(canon);
        }
    }
    let first = pass()?;
    state.after(first, &mut watcher);
    if announce {
        let roots: Vec<String> = state
            .roots
            .iter()
            .map(|r| r.display().to_string())
            .collect();
        eprintln!(
            "computed: watching {} template(s) under {}; Ctrl-C stops",
            state.templates.len(),
            roots.join(", ")
        );
    }
    loop {
        let mut changed: BTreeSet<PathBuf> = match rx.recv_timeout(QUIET) {
            Ok(e) => paths(e).into_iter().collect(),
            Err(RecvTimeoutError::Timeout) if STOP.load(Ordering::SeqCst) => return Ok(0),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Err("the watcher stopped".into()),
        };
        let began = Instant::now();
        while began.elapsed() < PATIENCE && !STOP.load(Ordering::SeqCst) {
            match rx.recv_timeout(QUIET) {
                Ok(e) => changed.extend(paths(e)),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Err("the watcher stopped".into()),
            }
        }
        if STOP.load(Ordering::SeqCst) {
            return Ok(0);
        }
        let lost = changed.contains(Path::new(""));
        // Every path is looked at, so each own write is checked off.
        let hits = changed.iter().filter(|p| state.relevant(p)).count();
        if hits == 0 && !lost {
            continue;
        }
        match pass() {
            Ok(p) => state.after(p, &mut watcher),
            Err(e) => eprintln!("computed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_writes_are_dropped_until_the_file_changes_again() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("a.md");
        std::fs::write(&file, "written").unwrap();
        let mut state = State {
            roots: [root.clone()].into(),
            ..State::default()
        };
        state.own.insert(file.clone(), sum(&file).unwrap());
        assert!(!state.relevant(&file));
        assert!(!state.relevant(&file), "a second event for the same write");
        std::fs::write(&file, "edited").unwrap();
        assert!(state.relevant(&file));
        assert!(!state.own.contains_key(&file));
    }

    #[test]
    fn ignored_paths_matter_only_when_a_snapshot_read_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        let mut state = State {
            roots: [root.clone()].into(),
            ..State::default()
        };
        assert!(state.relevant(&root.join("src/new.rs")));
        assert!(!state.relevant(&root.join("target/debug/out")));
        assert!(!state.relevant(&root.join(".git/index")));
        assert!(!state.relevant(&root.join(".computed-abc.tmp")));
        state.read.insert(root.join("target/debug/out"));
        assert!(state.relevant(&root.join("target/debug/out")));
    }

    #[test]
    fn outside_the_roots_only_templates_and_reads_matter() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut state = State {
            roots: [root.join("docs")].into(),
            ..State::default()
        };
        assert!(!state.relevant(&root.join("src/x.rs")));
        state.templates.insert(root.join("README.md"));
        assert!(state.relevant(&root.join("README.md")));
    }
}
