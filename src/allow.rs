//! The per-machine allowlist of URL prefixes a `remote` region may fetch:
//! `remote.toml` beside the trust store, never anything inside a working
//! tree. Independent of trust: a trusted repository allows no URL, and an
//! allowed URL needs no trust. `--allow` adds prefixes for one invocation.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fs;
use crate::trust::{self, Error};

/// A parsed `http(s)://host[:port]/path`: scheme and host lowercase, the
/// scheme's default port dropped, the path with `.` and `..` segments
/// resolved. Query and fragment are not kept.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Url {
    scheme: String,
    host: String,
    port: Option<u16>,
    path: String,
}

impl Url {
    /// Parses a URL; `prefix` refuses what an allowlist entry may not hold:
    /// a query, a fragment or a wildcard.
    fn parse(s: &str, prefix: bool) -> Result<Url, String> {
        let bad = |why: &str| format!("{s}: {why}");
        let (scheme, rest) = s
            .split_once("://")
            .ok_or_else(|| bad("expected http:// or https://"))?;
        let scheme = scheme.to_ascii_lowercase();
        let default_port = match scheme.as_str() {
            "https" => 443,
            "http" => 80,
            _ => return Err(bad("expected http:// or https://")),
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.contains('@') {
            return Err(bad("a URL with credentials is not allowed"));
        }
        let (host, port) = match authority.strip_prefix('[') {
            Some(v6) => {
                let (host, after) = v6.split_once(']').ok_or_else(|| bad("unclosed ["))?;
                if !after.is_empty() && !after.starts_with(':') {
                    return Err(bad("text after the ] of an IPv6 host"));
                }
                (format!("[{host}]"), after.strip_prefix(':'))
            }
            None => match authority.split_once(':') {
                Some((host, port)) => (host.to_string(), Some(port)),
                None => (authority.to_string(), None),
            },
        };
        if host.is_empty() {
            return Err(bad("no host"));
        }
        if prefix && host.contains('*') {
            return Err(bad(
                "a host is matched exactly; wildcards are not supported",
            ));
        }
        let port = match port {
            None | Some("") => None,
            Some(p) => Some(
                p.parse::<u16>()
                    .map_err(|_| bad("the port is not a number"))?,
            ),
        }
        .filter(|&p| p != default_port);
        let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
        if prefix && path_end < tail.len() {
            return Err(bad("a prefix has no query or fragment"));
        }
        if ambiguous(&tail[..path_end]) {
            return Err(bad(
                "servers read this path differently: it holds //, a \\, an encoded / or \\, or a segment that starts with .. and goes on",
            ));
        }
        Ok(Url {
            scheme,
            host: host.to_ascii_lowercase(),
            port,
            path: normalise_path(&tail[..path_end]),
        })
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme, self.host)?;
        if let Some(p) = self.port {
            write!(f, ":{p}")?;
        }
        f.write_str(&self.path)
    }
}

/// Whether servers disagree on what `path` names, so that no prefix can be
/// judged to cover it: an empty segment (`//`, which some merge before
/// resolving `..`), a `\` or an encoded `/` or `\` (which some read as a
/// separator), or a segment that starts with `..` and goes on (`..;`, which
/// some cut at the `;`).
fn ambiguous(path: &str) -> bool {
    let path = path.to_ascii_lowercase().replace("%2e", ".");
    path.contains("//")
        || path.contains('\\')
        || path.contains("%2f")
        || path.contains("%5c")
        || path.split('/').any(|s| s.starts_with("..") && s != "..")
}

/// The path a server would serve: `%2e` read as `.`, then `.` and `..`
/// segments resolved as RFC 3986 does, so `/org/../other` is `/other` and
/// cannot pass for something under `/org/`. Empty is `/`.
fn normalise_path(path: &str) -> String {
    let path = path.replace("%2e", ".").replace("%2E", ".");
    let mut out: Vec<&str> = Vec::new();
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    for (i, seg) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        match *seg {
            "." if last => out.push(""),
            "." => {}
            ".." => {
                out.pop();
                if last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    format!("/{}", out.join("/"))
}

/// One allowlist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix(Url);

impl Prefix {
    pub fn parse(s: &str) -> Result<Prefix, String> {
        Url::parse(s, true).map(Prefix)
    }

    /// Whether the entry covers `url`: scheme, host and port equal, and the
    /// entry's path a prefix of the url's at a `/`, so `https://h/org`
    /// covers `/org` and `/org/x` and not `/orgs`.
    fn covers(&self, url: &Url) -> bool {
        let p = &self.0;
        if p.scheme != url.scheme || p.host != url.host || p.port != url.port {
            return false;
        }
        if p.path.ends_with('/') {
            url.path.starts_with(&p.path)
        } else {
            url.path == p.path
                || url
                    .path
                    .strip_prefix(&p.path)
                    .is_some_and(|rest| rest.starts_with('/'))
        }
    }
}

impl fmt::Display for Prefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The prefixes in force for one invocation: the store's and `--allow`'s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Allowed {
    prefixes: Vec<Prefix>,
}

impl Allowed {
    pub fn new(prefixes: Vec<Prefix>) -> Allowed {
        Allowed { prefixes }
    }

    /// Whether some entry covers `url`. A url that does not parse is not.
    pub fn allows(&self, url: &str) -> bool {
        Url::parse(url, false).is_ok_and(|u| self.prefixes.iter().any(|p| p.covers(&u)))
    }
}

/// The prefix to suggest for `url`: its origin and the directory it sits
/// in. `Err` says why no prefix can cover it.
pub fn suggestion(url: &str) -> Result<String, String> {
    let mut u = Url::parse(url, false)?;
    let dir = u.path.rfind('/').map_or(0, |i| i + 1);
    u.path.truncate(dir);
    Ok(u.to_string())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Entries {
    #[serde(default)]
    prefixes: Vec<String>,
}

/// The store file, injectable for tests.
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn at(path: PathBuf) -> Store {
        Store { path }
    }

    /// `remote.toml` in the directory that holds `trust.toml`.
    pub fn default_path() -> Result<PathBuf, Error> {
        Ok(trust::Store::default_path()?.with_file_name("remote.toml"))
    }

    fn read(&self) -> Result<Entries, Error> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|e| Error(format!("{}: {e}", self.path.display())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Entries::default()),
            Err(e) => Err(Error(format!("{}: {e}", self.path.display()))),
        }
    }

    fn save(&self, entries: &Entries) -> Result<(), Error> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
        }
        let text = toml::to_string(entries).map_err(|e| Error(e.to_string()))?;
        fs::write(&self.path, &text).map_err(|e| Error(format!("{}: {e}", self.path.display())))?;
        Ok(())
    }

    /// The stored prefixes, in the order they were recorded.
    pub fn list(&self) -> Result<Vec<String>, Error> {
        Ok(self.read()?.prefixes)
    }

    /// Records `prefix` in its normal form and returns that form.
    pub fn allow(&self, prefix: &str) -> Result<String, Error> {
        let normal = Prefix::parse(prefix).map_err(Error)?.to_string();
        let mut entries = self.read()?;
        if !entries.prefixes.contains(&normal) {
            entries.prefixes.push(normal.clone());
            self.save(&entries)?;
        }
        Ok(normal)
    }

    /// Removes `prefix`, compared in normal form; `None` when it was not
    /// there, else the form removed.
    pub fn disallow(&self, prefix: &str) -> Result<Option<String>, Error> {
        let normal = Prefix::parse(prefix).map_err(Error)?.to_string();
        let mut entries = self.read()?;
        let before = entries.prefixes.len();
        entries.prefixes.retain(|p| p != &normal);
        if entries.prefixes.len() == before {
            return Ok(None);
        }
        self.save(&entries)?;
        Ok(Some(normal))
    }

    /// The stored prefixes and `extra`, the invocation's `--allow`.
    pub fn allowed(&self, extra: &[String]) -> Result<Allowed, Error> {
        let mut prefixes = Vec::new();
        for p in self.list()?.iter().chain(extra) {
            prefixes.push(Prefix::parse(p).map_err(Error)?);
        }
        Ok(Allowed::new(prefixes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers(prefix: &str, url: &str) -> bool {
        Allowed::new(vec![Prefix::parse(prefix).unwrap()]).allows(url)
    }

    #[test]
    fn a_prefix_covers_by_component_at_a_slash() {
        let table = [
            ("https://h/org/", "https://h/org/x", true),
            ("https://h/org/", "https://h/org", false),
            ("https://h/org", "https://h/org", true),
            ("https://h/org", "https://h/org/x/y.md", true),
            ("https://h/org", "https://h/orgs", false),
            ("https://h/org", "https://h/orgs/x", false),
            ("https://h", "https://h/anything", true),
            ("https://h/", "https://h", true),
            ("https://H.example/Org", "https://h.EXAMPLE/Org/x", true),
            ("https://h/Org", "https://h/org/x", false),
            ("https://h:443/org", "https://h/org/x", true),
            ("https://h/org", "https://h:8443/org/x", false),
            ("https://h/org", "http://h/org/x", false),
            ("https://h/org", "https://g/org/x", false),
            ("https://h/org", "https://h.evil/org/x", false),
            ("https://h/org", "https://h/org/x?y=/z", true),
            ("https://h/org", "https://h/org/../secret", false),
            ("https://h/org", "https://h/org/%2e%2e/secret", false),
            ("https://h/org", "https://h/org/./x", true),
            ("https://h/org", "https://user@h/org/x", false),
            ("http://127.0.0.1:8080/", "http://127.0.0.1:8080/a", true),
            ("http://127.0.0.1:8080/", "http://127.0.0.1:9090/a", false),
            ("http://[::1]:9/", "http://[::1]:9/a", true),
            // Paths servers read differently are covered by nothing.
            ("https://h/org/", "https://h/org//../secret", false),
            ("https://h/org/", "https://h/org/..%2fsecret", false),
            ("https://h/org/", "https://h/org/..%2Fsecret", false),
            ("https://h/org/", "https://h/org/..%5csecret", false),
            ("https://h/org/", "https://h/org/..\\secret", false),
            ("https://h/org/", "https://h/org/..;/secret", false),
            ("https://h/org/", "https://h/org/%2e%2e;/secret", false),
            ("https://h/org/", "https://h/org/a..b/x", true),
            // The authority is one host and at most one port.
            ("http://[::1]/", "http://[::1]evil.example/x", false),
            ("http://127.0.0.1:9/", "http://127.0.0.1:9:80/x", false),
        ];
        for (prefix, url, want) in table {
            assert_eq!(covers(prefix, url), want, "{prefix} covers {url}");
        }
    }

    #[test]
    fn a_prefix_is_normalised_and_refuses_what_it_cannot_mean() {
        assert_eq!(
            Prefix::parse("HTTPS://Raw.Example:443/a/./b/../c")
                .unwrap()
                .to_string(),
            "https://raw.example/a/c"
        );
        assert_eq!(
            Prefix::parse("https://h").unwrap().to_string(),
            "https://h/"
        );
        for (bad, why) in [
            ("ftp://h/", "expected http:// or https://"),
            ("h/org", "expected http:// or https://"),
            ("https://*.h/", "wildcards are not supported"),
            ("https://h/a?b=c", "no query or fragment"),
            ("https://h/a#b", "no query or fragment"),
            ("https:///a", "no host"),
            ("https://h:x/", "port"),
            ("https://u@h/", "credentials"),
            ("https://h/a//b/", "servers read"),
            ("https://[::1]x/", "after the ]"),
            ("https://h:1:2/", "port"),
        ] {
            let e = Prefix::parse(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn the_suggestion_is_the_origin_and_directory() {
        assert_eq!(
            suggestion("https://Raw.Example/org/repo/main/README.md?x=1").unwrap(),
            "https://raw.example/org/repo/main/"
        );
        assert_eq!(suggestion("https://h").unwrap(), "https://h/");
        assert!(
            suggestion("https://u@h/x")
                .unwrap_err()
                .contains("credentials")
        );
    }

    #[test]
    fn allow_disallow_and_list_round_trip_through_the_store_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/computed/remote.toml");
        let store = Store::at(path.clone());
        assert!(store.list().unwrap().is_empty());
        assert_eq!(
            store.allow("HTTPS://H/org").unwrap(),
            "https://h/org",
            "recorded in normal form"
        );
        assert_eq!(store.allow("https://h:443/org").unwrap(), "https://h/org");
        store.allow("https://g/").unwrap();
        assert_eq!(store.list().unwrap(), ["https://h/org", "https://g/"]);
        let allowed = store.allowed(&["https://x/y".to_string()]).unwrap();
        assert!(allowed.allows("https://h/org/a") && allowed.allows("https://x/y/z"));
        assert!(!allowed.allows("https://h/other"));
        assert_eq!(
            store.disallow("https://H/org").unwrap().as_deref(),
            Some("https://h/org")
        );
        assert_eq!(store.disallow("https://h/org").unwrap(), None);
        assert_eq!(store.list().unwrap(), ["https://g/"]);
        assert!(store.allowed(&["nope".to_string()]).is_err());
        std::fs::write(&path, "prefixes = [").unwrap();
        assert!(store.list().is_err());
    }
}
