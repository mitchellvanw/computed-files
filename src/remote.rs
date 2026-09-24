//! The `remote` loader: a document fetched over HTTPS and pinned by the
//! SHA-256 of its bytes in the opener. The snapshot is the url and the pin,
//! so `check` never touches the network; `run` fetches, and a body that no
//! longer matches the pin is a loader failure. `computed update` moves pins.
//! Every url fetched, redirects included, must be on this machine's
//! allowlist (`crate::allow`).

use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::allow::{self, Allowed};
use crate::loader::{LoadError, Loaded};
use crate::marker::Opener;

/// The most a fetched document may weigh.
const LIMIT: u64 = 10 * 1024 * 1024;

/// The most redirects one fetch follows.
const REDIRECTS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteArgs {
    pub url: String,
    /// The pinned SHA-256, 64 lowercase hex characters; `None` until
    /// `computed update` pins it.
    pub sha256: Option<String>,
    pub timeout: Duration,
}

impl RemoteArgs {
    pub fn from_opener(opener: &Opener) -> Result<RemoteArgs, LoadError> {
        validate(opener).map_err(LoadError::Hard)?;
        Ok(RemoteArgs {
            url: opener.attr("url").expect("validated").to_string(),
            sha256: opener.attr("sha256").map(str::to_string),
            timeout: Duration::from_secs(
                opener
                    .attr("timeout")
                    .map_or(30, |t| t.parse().expect("validated")),
            ),
        })
    }
}

/// The opener rules the grammar checks: `url=` required and allowed,
/// `sha256=` a full lowercase digest, `timeout=` whole seconds of at least 1.
pub fn validate(opener: &Opener) -> Result<(), String> {
    let Some(url) = opener.attr("url") else {
        return Err("remote needs url=".to_string());
    };
    if !scheme_ok(url) {
        return Err(format!(
            "url={url}: expected https://, or http:// to localhost"
        ));
    }
    if let Some(pin) = opener.attr("sha256")
        && !is_digest(pin)
    {
        return Err(format!(
            "sha256={pin}: expected 64 lowercase hex characters"
        ));
    }
    match opener.attr("timeout") {
        Some(t) if t.parse::<u64>().map_or(true, |t| t == 0) => Err(format!(
            "timeout={t}: expected seconds as a whole number of at least 1"
        )),
        _ => Ok(()),
    }
}

/// `https://` anywhere, and plain `http://` only to this machine, where a
/// test or a local server can stand in.
fn scheme_ok(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn is_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The SHA-256 of `bytes` as 64 lowercase hex characters.
pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `url NUL sha256`: the pin stands for the content, so no fetch is needed
/// to know whether the region is fresh.
pub fn snapshot(args: &RemoteArgs) -> Vec<u8> {
    let mut out = args.url.as_bytes().to_vec();
    out.push(0);
    out.extend_from_slice(args.sha256.as_deref().unwrap_or("").as_bytes());
    out
}

/// Fetches the url and checks the body against the pin. A url the
/// allowlist does not cover is not fetched. An unpinned region, a failed
/// fetch, a body that does not match the pin or is not UTF-8: each is a
/// loader failure, and the previous body is kept.
pub fn load(args: &RemoteArgs, allowed: &Allowed) -> Result<Loaded, LoadError> {
    let failed = |stderr: String| LoadError::Failed { stderr };
    if !allowed.allows(&args.url) {
        return Err(LoadError::NotAllowed(not_allowed(&args.url)));
    }
    let Some(pin) = &args.sha256 else {
        return Err(failed(
            "no sha256= pin; run `computed update` to fetch the url and pin it".to_string(),
        ));
    };
    let body = fetch(&args.url, args.timeout, allowed).map_err(failed)?;
    let got = digest(&body);
    if &got != pin {
        return Err(failed(format!(
            "the fetched body does not match the pin\npinned  {pin}\nfetched {got}\nrun `computed update` if the change is expected"
        )));
    }
    let text = String::from_utf8(body).map_err(|_| failed("the body is not UTF-8".to_string()))?;
    Ok(Loaded {
        text,
        snapshot: snapshot(args),
    })
}

/// Why a url is skipped, and the prefix that would allow it.
pub fn not_allowed(url: &str) -> String {
    let prefix = allow::suggestion(url);
    format!(
        "{url} is not on this machine's allowlist; `computed allow {prefix}` allows it, or `--allow {prefix}` for one invocation"
    )
}

/// The body at `url`, up to [`LIMIT`] bytes, within `timeout` in all. A
/// redirect is followed only to a url the allowlist covers and the scheme
/// rule permits, at most [`REDIRECTS`] of them; a status other than 2xx is
/// an error. `url` itself must already be allowed.
pub fn fetch(url: &str, timeout: Duration, allowed: &Allowed) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + timeout;
    let mut url = url.to_string();
    for _ in 0..=REDIRECTS {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("{url}: timed out after {}s", timeout.as_secs()));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(left))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into();
        let mut response = agent.get(&url).call().map_err(|e| format!("{url}: {e}"))?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|l| l.to_str().ok())
                .ok_or_else(|| format!("{url}: {status} without a Location"))?;
            let next = join(&url, location);
            if !scheme_ok(&next) {
                return Err(format!(
                    "{url} redirects to {next}: expected https://, or http:// to localhost"
                ));
            }
            if !allowed.allows(&next) {
                return Err(format!("{url} redirects to {}", not_allowed(&next)));
            }
            url = next;
            continue;
        }
        if !status.is_success() {
            return Err(format!("{url}: {status}"));
        }
        return response
            .body_mut()
            .with_config()
            .limit(LIMIT)
            .read_to_vec()
            .map_err(|e| format!("{url}: {e}"));
    }
    Err(format!("{url}: more than {REDIRECTS} redirects"))
}

/// A `Location` resolved against the url it came from.
fn join(base: &str, location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    if let Some(authority_path) = location.strip_prefix("//") {
        return format!("{scheme}://{authority_path}");
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let origin = &base[..scheme.len() + 3 + authority_end];
    if location.starts_with('/') {
        return format!("{origin}{location}");
    }
    let path = &rest[authority_end..];
    let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    let dir = if dir.is_empty() { "/" } else { dir };
    format!("{origin}{dir}{location}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::{self, Segment};

    fn opener(attrs: &str) -> Result<Opener, String> {
        let text = format!("<!-- computed remote {attrs} -->\n<!-- /computed -->\n");
        let file = marker::parse(&text).map_err(|e| e.message)?;
        match file.segments.into_iter().next() {
            Some(Segment::Region(r)) => Ok(r.opener),
            _ => unreachable!(),
        }
    }

    const PIN: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn the_opener_takes_https_and_a_full_pin() {
        let o = opener(&format!(
            "url=https://example.com/a.md sha256={PIN} timeout=5"
        ))
        .unwrap();
        let args = RemoteArgs::from_opener(&o).unwrap();
        assert_eq!(args.sha256.as_deref(), Some(PIN));
        assert_eq!(args.timeout, Duration::from_secs(5));
        assert_eq!(
            snapshot(&args),
            format!("https://example.com/a.md\0{PIN}").into_bytes()
        );
        for url in [
            "http://127.0.0.1:8080/x",
            "http://localhost/x",
            "http://[::1]:9/x",
        ] {
            assert!(opener(&format!("url={url}")).is_ok(), "{url}");
        }
        for (attrs, message) in [
            ("sha256=abc", "remote needs url="),
            (
                "url=http://example.com/x",
                "url=http://example.com/x: expected https://, or http:// to localhost",
            ),
            (
                "url=http://127.0.0.1.example.com/x",
                "url=http://127.0.0.1.example.com/x: expected https://, or http:// to localhost",
            ),
            (
                "url=https://a sha256=ABC",
                "sha256=ABC: expected 64 lowercase hex characters",
            ),
            (
                "url=https://a timeout=0",
                "timeout=0: expected seconds as a whole number of at least 1",
            ),
        ] {
            assert_eq!(opener(attrs).unwrap_err(), message, "{attrs}");
        }
    }

    #[test]
    fn an_unpinned_or_disallowed_region_fails_without_fetching() {
        let args =
            RemoteArgs::from_opener(&opener("url=https://example.invalid/a/x").unwrap()).unwrap();
        assert_eq!(
            load(&args, &Allowed::default()),
            Err(LoadError::NotAllowed(
                "https://example.invalid/a/x is not on this machine's allowlist; `computed allow https://example.invalid/a/` allows it, or `--allow https://example.invalid/a/` for one invocation".to_string()
            ))
        );
        let allowed = Allowed::new(vec![
            allow::Prefix::parse("https://example.invalid/").unwrap(),
        ]);
        assert!(matches!(
            load(&args, &allowed),
            Err(LoadError::Failed { stderr }) if stderr.contains("computed update")
        ));
    }

    #[test]
    fn a_location_resolves_against_the_url_it_came_from() {
        let base = "https://h/a/b.md?q=1";
        for (location, want) in [
            ("https://g/x", "https://g/x"),
            ("//g/x", "https://g/x"),
            ("/x", "https://h/x"),
            ("c.md", "https://h/a/c.md"),
            ("../c.md", "https://h/a/../c.md"),
        ] {
            assert_eq!(join(base, location), want, "{location}");
        }
        assert_eq!(join("http://127.0.0.1:9", "x"), "http://127.0.0.1:9/x");
    }
}
