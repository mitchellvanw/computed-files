//! The `remote` loader: a document fetched over HTTPS and pinned by the
//! SHA-256 of its bytes in the opener. The snapshot is the url and the pin,
//! so `check` never touches the network; `run` fetches, and a body that no
//! longer matches the pin is a loader failure. `computed update` moves pins.

use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::loader::{LoadError, Loaded};
use crate::marker::Opener;

/// The most a fetched document may weigh.
const LIMIT: u64 = 10 * 1024 * 1024;

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
    allowed(url)?;
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
fn allowed(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = match host.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => host.split(':').next().unwrap_or(""),
        };
        if matches!(host, "localhost" | "127.0.0.1" | "::1") {
            return Ok(());
        }
    }
    Err(format!(
        "url={url}: expected https://, or http:// to localhost"
    ))
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

/// Fetches the url and checks the body against the pin. An unpinned
/// region, a failed fetch, a body that does not match the pin or is not
/// UTF-8: each is a loader failure, and the previous body is kept.
pub fn load(args: &RemoteArgs) -> Result<Loaded, LoadError> {
    let failed = |stderr: String| LoadError::Failed { stderr };
    let Some(pin) = &args.sha256 else {
        return Err(failed(
            "no sha256= pin; run `computed update` to fetch the url and pin it".to_string(),
        ));
    };
    let body = fetch(&args.url, args.timeout).map_err(failed)?;
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

/// The body at `url`, redirects followed, up to [`LIMIT`] bytes. A status
/// other than 2xx is an error. An `https://` url never follows a redirect
/// to plain `http://`.
pub fn fetch(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .https_only(url.starts_with("https://"))
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
    response
        .body_mut()
        .with_config()
        .limit(LIMIT)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))
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
    fn an_unpinned_region_fails_without_fetching() {
        let args =
            RemoteArgs::from_opener(&opener("url=https://example.invalid/x").unwrap()).unwrap();
        assert!(matches!(
            load(&args),
            Err(LoadError::Failed { stderr }) if stderr.contains("computed update")
        ));
    }
}
