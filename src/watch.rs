use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use crate::ops;
use crate::report;

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut map = BTreeMap::new();
    snap(root, &mut map);
    map
}

fn snap(dir: &Path, map: &mut BTreeMap<PathBuf, Vec<u8>>) {
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" || name.ends_with(".tmp") {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            snap(&p, map);
        } else if let Ok(bytes) = fs::read(&p) {
            map.insert(p, bytes);
        }
    }
}

fn diff(prev: &BTreeMap<PathBuf, Vec<u8>>, cur: &BTreeMap<PathBuf, Vec<u8>>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for (k, v) in cur {
        match prev.get(k) {
            None => out.push(k.clone()),
            Some(p) if p != v => out.push(k.clone()),
            _ => {}
        }
    }
    for k in prev.keys() {
        if !cur.contains_key(k) {
            out.push(k.clone());
        }
    }
    out
}

pub fn watch(root: &Path, tmpl: &Path, out: &Path) {
    let mut prev = snapshot(root);
    println!(
        "watching {} · baseline {} files · poll 250 ms · settle 150 ms · Ctrl-C to stop",
        root.display(),
        prev.len()
    );
    let mut last_written: Option<(PathBuf, String)> = None;

    loop {
        sleep(Duration::from_millis(250));
        let mut cur = snapshot(root);
        let mut changed = diff(&prev, &cur);
        if changed.is_empty() {
            prev = cur;
            continue;
        }
        for _ in 0..6 {
            sleep(Duration::from_millis(150));
            let c2 = snapshot(root);
            if c2 == cur {
                break;
            }
            cur = c2;
            changed = diff(&prev, &cur);
        }

        for path in &changed {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            if path == out {
                let content = fs::read_to_string(path).ok();
                let own = matches!(
                    &last_written,
                    Some((p, t)) if p == path && content.as_deref() == Some(t.as_str())
                );
                if own {
                    println!("event {} → matches our last write, dropped (own-write guard)", rel);
                    continue;
                }
                println!("event {} → changed by someone else; check, don't overwrite", rel);
                match ops::check_once(root, tmpl, out) {
                    None => println!("check → exit 1 · {} is missing", out.display()),
                    Some(o) => report::print_report("check", o.exit, None, &o.report),
                }
            } else {
                println!("event {} → template or input changed; rendering", rel);
                let o = ops::run_once(root, tmpl, out, false);
                report::print_report("run", o.exit, Some(o.wrote), &o.report);
                if o.wrote {
                    last_written = Some((out.to_path_buf(), o.text));
                }
            }
        }
        prev = cur;
    }
}
