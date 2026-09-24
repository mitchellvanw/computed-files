---
status: accepted
---

# Remote regions are pinned by SHA-256 and fetch only under a per-machine allowlist

A `remote` region renders a document fetched over HTTPS: `remote url=https://… sha256=<hex>`. The pin is the SHA-256 of the bytes it renders. The snapshot is the url and the pin, so `check` never touches the network. `run` fetches, and a body that does not match the pin is a loader failure that keeps the old body. `computed update` fetches every remote region's url and rewrites its `sha256=` in place, and renders nothing: the moved pin changes the opener, the region is stale, and the next `run` fetches and renders it.

Every url a region fetches, redirects included, must lie under a prefix on this machine's allowlist. `computed allow PREFIX` records one in `remote.toml` beside `trust.toml`, never in the working tree. `--allow PREFIX` allows one for a single invocation. A region whose url is not allowed is reported `disallowed`, keeps its body and exits 1, as an untrusted exec region does. The allowlist and trust are independent.

## Context

Documents shared between repositories are a common need: an organisation's agent rules, a style guide, a licence header. The only way to include one was `exec cmd="curl …"`. That needs trust, and it is either `volatile`, so `check` never sees a change, or it declares `inputs=` that say nothing about the url. It also fetches on every render.

Three forces pull against each other. `check` has to stay offline and deterministic ([ADR 0006](0006-check-never-runs-a-loader.md)). A change to a shared document should be a reviewed diff, not a silent re-render. And a fetch is a request sent from the developer's machine to a host the repository names. A pull request can add a url that points at an internal service, a cloud metadata address or a tracker.

## Considered Options

- **Snapshot the fetched body.** The region would go stale the moment the document changes. Rejected: `check` would hit the network on every commit, be as slow and flaky as the host, and let any clone make CI send requests.
- **A `volatile` remote.** Rejected: `check` never sees a change, and every `run` fetches.
- **Pin by `ETag` or `Last-Modified`.** No hashing. Rejected: the server controls both, and neither names the content.
- **`update` renders too,** so a pin move costs one fetch rather than two. Rejected: moving a pin and rendering are separate decisions. Keeping `update` to the opener line keeps its diff one line per region, and keeps `run` the only thing that writes a body.
- **Gate fetching with trust.** No second store. Rejected: a grant answers "may this clone run code here". Under it, a trusted clone would fetch any url a pulled branch names, internal hosts included. And a pinned document, which runs nothing, would need a grant.
- **The pin is the whole defence.** The content is verified, so any url is fine. Rejected: the pin protects what is rendered, not the request. The request is where the harm is.
- **An allowlist committed in the repository.** Rejected for the reason ADR 0007 rejected a committed trust file: the attacker controls it.
- **A per-machine allowlist of url prefixes, separate from trust.** Chosen.

## Consequences

- On a fresh clone, pinned remote regions are fresh under `check` with no network and no allowlist. `run` reports them `disallowed` until someone allows their prefix.
- A new remote region with no pin is `unrendered`. `run` does not fetch it and says to run `computed update`.
- Prefixes are compared parsed, never as strings. Scheme and host are compared without case, a default port is dropped, and `.`/`..` and `%2e` segments are resolved before the path is matched at a `/` boundary, so `https://h/org/` covers `/org/x` and not `/orgs`. Entries with credentials, a query, a fragment or a wildcard host are refused.
- Redirects are followed by hand, at most ten, each one checked against the scheme rule and the allowlist.
- Plain `http://` is allowed only to `localhost`, `127.0.0.1` and `[::1]`. Bodies are capped at 10 MiB. Proxies come from the environment.
- The binary carries an HTTP client, ureq with rustls, and no system TLS library.
- The GitHub Action takes an `allow` input for `suggest`, passed as `--allow`. The runner keeps no allowlist.
- `update` does not write into `computed.toml`. For a `use` region whose recipe is a remote, it reports the new pin for the recipe, as a tier-2 answer.

## Amended

Resolving `.` and `..` is not enough where servers disagree on what a path names. A path holding `//`, `\`, `%2f` or `%5c`, or a segment that starts with `..` and goes on, such as `..;`, is covered by no prefix, and a prefix holding one is refused. So is text after an IPv6 host's `]`, or a second `:port`. For such a url the `disallowed` message says no prefix can allow it, and why. A redirect's `Location` is absolute only when a scheme and `://` start it, so one with `://` only in its query resolves against the url it came from.
