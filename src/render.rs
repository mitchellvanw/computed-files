//! Decides what every region becomes. Pure: a parsed file, a mode, a trust
//! flag and a `Loaders` seam in; the new text and a report per region out.
//! Owns the sums, the freshness cache, the states, the refuse rule and its
//! per-file consequence, the untrusted skip, loader failure keeping the
//! body, and `clean`.

use std::fmt;

use crate::loader::{self, LoadError, Loaded};
use crate::marker::{self, File, OnStale, Region, Segment, Sums};
use crate::sink;

/// The seam between `render` and the loaders. `snapshot` costs nothing
/// dangerous and always runs; `load` may run a command and runs only when
/// the region is stale, unrendered or volatile.
pub trait Loaders {
    /// The snapshot of the region's inputs; `None` when the region is volatile.
    fn snapshot(&mut self, region: &Region) -> Result<Option<Vec<u8>>, LoadError>;
    fn load(&mut self, region: &Region) -> Result<Loaded, LoadError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Run { force: bool },
    DryRun { force: bool },
    Check,
    Clean { force: bool, dry_run: bool },
}

impl Mode {
    /// Whether the mode prints a diff instead of writing.
    pub fn dry_run(self) -> bool {
        matches!(
            self,
            Mode::DryRun { .. } | Mode::Clean { dry_run: true, .. }
        )
    }

    fn force(self) -> bool {
        match self {
            Mode::Run { force } | Mode::DryRun { force } | Mode::Clean { force, .. } => force,
            Mode::Check => false,
        }
    }
}

/// A region's freshness, derived from the file and its inputs alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Fresh,
    Stale,
    Edited,
    StaleEdited,
    Volatile,
    Unrendered,
    /// The tool could not answer: the snapshot was a hard error.
    Error,
}

impl State {
    fn edited(self) -> bool {
        matches!(self, State::Edited | State::StaleEdited)
    }
    /// Whether `check` reports drift for this state.
    pub fn drifted(self) -> bool {
        !matches!(self, State::Fresh | State::Volatile)
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            State::Fresh => "fresh",
            State::Stale => "stale",
            State::Edited => "edited",
            State::StaleEdited => "stale+edited",
            State::Volatile => "volatile",
            State::Unrendered => "unrendered",
            State::Error => "error",
        })
    }
}

/// What happened to a region, shown in the report's last column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A fresh region, left as it was.
    Fresh,
    Written,
    WouldWrite,
    /// Left untouched because the file was refused.
    Kept,
    Refused,
    Untrusted,
    Failed,
    /// Tier 2: the tool could not answer for this region; body kept.
    Error,
    Cleaned,
    WouldClean,
    /// Under `check`: a stale region whose opener says `on-stale=warn`,
    /// reported without failing.
    Warn,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Action::Fresh => "",
            Action::Written => "written",
            Action::WouldWrite => "would write",
            Action::Kept => "kept",
            Action::Refused => "refused; run with --force",
            Action::Untrusted => "skipped; run `computed trust`",
            Action::Failed => "failed; body kept",
            Action::Error => "skipped; body kept",
            Action::Cleaned => "cleaned",
            Action::WouldClean => "would clean",
            Action::Warn => "warn",
        })
    }
}

impl Action {
    /// The action's name in machine-readable output.
    pub fn key(self) -> &'static str {
        match self {
            Action::Fresh => "fresh",
            Action::Written => "written",
            Action::WouldWrite => "would-write",
            Action::Kept => "kept",
            Action::Refused => "refused",
            Action::Untrusted => "untrusted",
            Action::Failed => "failed",
            Action::Error => "error",
            Action::Cleaned => "cleaned",
            Action::WouldClean => "would-clean",
            Action::Warn => "warn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionReport {
    pub line: usize,
    pub name: Option<String>,
    pub loader: String,
    pub state: State,
    /// `None` under `check`, which has no action column.
    pub action: Option<Action>,
    /// Loader stderr, or the message of a region the tool could not answer.
    pub stderr: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendered {
    /// The file would change; `text` is what to write.
    Written {
        text: String,
        regions: Vec<RegionReport>,
    },
    Unchanged {
        regions: Vec<RegionReport>,
    },
    /// A hand-edited region refused the whole file.
    Refused {
        regions: Vec<RegionReport>,
    },
    /// Tier 2: the file is skipped whole.
    Error {
        line: usize,
        message: String,
    },
}

impl Rendered {
    pub fn regions(&self) -> &[RegionReport] {
        match self {
            Rendered::Written { regions, .. }
            | Rendered::Unchanged { regions }
            | Rendered::Refused { regions } => regions,
            Rendered::Error { .. } => &[],
        }
    }

    /// The exit tier this file contributes: 2 for an error, the file's or a
    /// region's; 1 when the content said no (a write, a refusal, a failure,
    /// an untrusted region, or drift under `check`); else 0.
    pub fn tier(&self) -> u8 {
        let errored = self
            .regions()
            .iter()
            .any(|r| r.state == State::Error || r.action == Some(Action::Error));
        if errored {
            return 2;
        }
        match self {
            Rendered::Error { .. } => 2,
            Rendered::Written { .. } | Rendered::Refused { .. } => 1,
            Rendered::Unchanged { regions } => {
                let said_no = regions.iter().any(|r| match r.action {
                    Some(Action::Untrusted | Action::Failed) => true,
                    Some(_) => false,
                    None => r.state.drifted(),
                });
                u8::from(said_no)
            }
        }
    }
}

mod sum {
    use crate::marker::Opener;
    use sha2::{Digest, Sha256};

    const DOMAIN: &str = "computed-in/1\n";

    fn hex(hash: impl AsRef<[u8]>) -> String {
        hash.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The indentation joins the loader line only when there is some, so
    /// the sum of an unindented region is what it always was.
    pub fn input(opener: &Opener, indent: &str, format_constant: u32, snapshot: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(DOMAIN.as_bytes());
        let indent = if indent.is_empty() {
            String::new()
        } else {
            format!(" indent={indent:?}")
        };
        h.update(format!("{}/{format_constant}{indent}\n", opener.loader).as_bytes());
        h.update(opener.canonical().as_bytes());
        h.update(b"\n");
        h.update(snapshot);
        hex(h.finalize())
    }

    pub fn output(body: &str) -> String {
        hex(Sha256::digest(body.as_bytes()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::marker::parse;

        #[test]
        fn output_sum_is_the_full_sha256_of_the_body() {
            assert_eq!(
                output(""),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            );
            assert_eq!(
                output("abc"),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
        }

        #[test]
        fn input_sum_is_the_full_sha256_of_the_domain_opener_and_snapshot() {
            let file = parse("<!-- computed tree -->\n<!-- /computed -->\n").unwrap();
            let opener = match &file.segments[0] {
                crate::marker::Segment::Region(r) => &r.opener,
                _ => unreachable!(),
            };
            // sha256("computed-in/1\ntree/1\n<!-- computed tree -->\n" + "snap")
            assert_eq!(
                input(opener, "", 1, b"snap"),
                "606574c24273299a1ee6b2e45e8ca207180ad978ea09dc5f8cf8a42631d41a1a"
            );
            // sha256("computed-in/1\ntree/1 indent=\"  \"\n<!-- computed tree -->\n" + "snap")
            assert_ne!(
                input(opener, "  ", 1, b"snap"),
                input(opener, "", 1, b"snap")
            );
        }
    }
}

fn state_of(region: &Region, snapshot: Option<&[u8]>) -> State {
    let Some(sums) = &region.sums else {
        return State::Unrendered;
    };
    let body_ok = sum::output(&region.body) == sums.output;
    match snapshot {
        None => {
            if body_ok {
                State::Volatile
            } else {
                State::Edited
            }
        }
        Some(snapshot) => {
            let constant = loader::format_constant(&region.opener.loader);
            let input_ok =
                sum::input(&region.opener, &region.indent, constant, snapshot) == sums.input;
            match (input_ok, body_ok) {
                (true, true) => State::Fresh,
                (false, true) => State::Stale,
                (true, false) => State::Edited,
                (false, false) => State::StaleEdited,
            }
        }
    }
}

/// The action `check` gives a region: `warn` for one that is only stale and
/// says `on-stale=warn`, else none. Every other drift still fails.
fn warned(region: &Region, state: State) -> Option<Action> {
    (state == State::Stale && region.opener.on_stale == OnStale::Warn).then_some(Action::Warn)
}

/// The state `clean` can know without a snapshot: only the body is tested.
fn body_state(region: &Region) -> State {
    match &region.sums {
        None => State::Unrendered,
        Some(sums) if sum::output(&region.body) == sums.output => State::Fresh,
        Some(_) => State::Edited,
    }
}

fn report(
    region: &Region,
    state: State,
    action: Option<Action>,
    stderr: Option<String>,
) -> RegionReport {
    RegionReport {
        line: region.line,
        name: region.opener.name.clone(),
        loader: region.opener.loader.clone(),
        state,
        action,
        stderr,
    }
}

fn raw_lines(region: &Region) -> String {
    format!("{}{}{}", region.raw_opener, region.body, region.raw_closer)
}

/// The opener's own line ending, which the tool keeps for every line it
/// writes into the region, so a CRLF file stays CRLF.
fn eol(region: &Region) -> &'static str {
    if region.raw_opener.ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The closer's own terminator, so a file that ends without a newline stays so.
fn closer_terminator(region: &Region) -> &str {
    let trimmed = region.raw_closer.trim_end_matches(['\n', '\r']);
    &region.raw_closer[trimmed.len()..]
}

/// A sink's LF body as it sits in the file: each non-blank line carries the
/// opener's indentation, so a region inside a list item stays inside it, and
/// every line ends as the opener does.
fn shape(region: &Region, body: &str) -> String {
    let eol = eol(region);
    let mut out = String::with_capacity(body.len());
    for line in body.split_inclusive('\n') {
        let text = line.strip_suffix('\n').unwrap_or(line);
        if !text.is_empty() {
            out.push_str(&region.indent);
            out.push_str(text);
        }
        out.push_str(eol);
    }
    out
}

fn region_text(region: &Region, opener_line: &str, body: &str, closer_line: &str) -> String {
    format!(
        "{indent}{opener_line}{eol}{body}{indent}{closer_line}{term}",
        indent = region.indent,
        eol = eol(region),
        term = closer_terminator(region)
    )
}

/// Renders one parsed file under `mode`. Performs no I/O.
pub fn file(parsed: &File, mode: Mode, trusted: bool, loaders: &mut dyn Loaders) -> Rendered {
    file_where(parsed, mode, trusted, &|_| true, loaders)
}

/// Renders the regions `select` picks. The others are reproduced byte for
/// byte and reported nowhere: they neither refuse the file nor move its tier.
pub fn file_where(
    parsed: &File,
    mode: Mode,
    trusted: bool,
    select: &dyn Fn(&Region) -> bool,
    loaders: &mut dyn Loaders,
) -> Rendered {
    let regions: Vec<&Region> = parsed
        .segments
        .iter()
        .filter_map(|s| match s {
            Segment::Region(r) => Some(r),
            Segment::Prose(_) => None,
        })
        .collect();

    // Snapshots first: they always run. One that is a hard error takes its
    // region out of the render and leaves the others to it.
    let mut states: Vec<Option<Result<State, String>>> = Vec::with_capacity(regions.len());
    for region in &regions {
        if !select(region) {
            states.push(None);
            continue;
        }
        if matches!(mode, Mode::Clean { .. }) {
            states.push(Some(Ok(body_state(region))));
            continue;
        }
        let state = match loaders.snapshot(region) {
            Ok(s) => Ok(state_of(region, s.as_deref())),
            Err(LoadError::Hard(message) | LoadError::Failed { stderr: message }) => Err(message),
        };
        states.push(Some(state));
    }
    let errored = |region: &Region, message: &String| {
        let action = (mode != Mode::Check).then_some(Action::Error);
        report(region, State::Error, action, Some(message.clone()))
    };

    if mode == Mode::Check {
        let reports = regions
            .iter()
            .zip(&states)
            .filter_map(|(r, s)| match s.as_ref()? {
                Ok(s) => Some(report(r, *s, warned(r, *s), None)),
                Err(m) => Some(errored(r, m)),
            })
            .collect();
        return Rendered::Unchanged { regions: reports };
    }

    // The hand-edit policy: one edited region refuses the whole file.
    let edited = states
        .iter()
        .any(|s| matches!(s, Some(Ok(s)) if s.edited()));
    if !mode.force() && edited {
        let reports = regions
            .iter()
            .zip(&states)
            .filter_map(|(r, s)| match s.as_ref()? {
                Ok(s) if s.edited() => Some(report(r, *s, Some(Action::Refused), None)),
                Ok(s) => Some(report(r, *s, Some(Action::Kept), None)),
                Err(m) => Some(errored(r, m)),
            })
            .collect();
        return Rendered::Refused { regions: reports };
    }

    let mut text = String::new();
    let mut reports = Vec::with_capacity(regions.len());
    let mut pieces = Vec::with_capacity(regions.len());
    let mut states = states.into_iter();
    for segment in &parsed.segments {
        let region = match segment {
            Segment::Prose(p) => {
                text.push_str(p);
                continue;
            }
            Segment::Region(r) => r,
        };
        let (piece, rep) = match states.next().expect("one state per region") {
            None => (raw_lines(region), None),
            Some(Err(m)) => (raw_lines(region), Some(errored(region, &m))),
            Some(Ok(state)) => {
                let (piece, mut rep) = match mode {
                    Mode::Clean { .. } => clean(region, state),
                    Mode::Run { force } | Mode::DryRun { force } => {
                        render(region, state, force, trusted, loaders)
                    }
                    Mode::Check => unreachable!("check returned above"),
                };
                // A region reproduced byte for byte, such as a volatile one
                // whose command printed the same text, has nothing to report.
                if piece == raw_lines(region)
                    && matches!(rep.action, Some(Action::Written | Action::Cleaned))
                {
                    rep.action = Some(Action::Fresh);
                }
                (piece, Some(rep))
            }
        };
        text.push_str(&piece);
        pieces.push(piece);
        reports.extend(rep);
    }

    let original = marker::serialise(parsed);
    if text == original {
        return Rendered::Unchanged { regions: reports };
    }
    if let Err((line, message)) = parses_back(&text, &original, &regions, &pieces) {
        return Rendered::Error { line, message };
    }
    let dry = mode.dry_run();
    for r in &mut reports {
        r.action = match (r.action, dry) {
            (Some(Action::Written), true) => Some(Action::WouldWrite),
            (Some(Action::Cleaned), true) => Some(Action::WouldClean),
            (a, _) => a,
        };
    }
    Rendered::Written {
        text,
        regions: reports,
    }
}

/// The invariant that a file the tool wrote always parses, checked on the
/// whole file: a region's fence can close one the prose left open above it,
/// which no check of the region alone can see. On failure, the line to fix
/// and why.
fn parses_back(
    text: &str,
    original: &str,
    regions: &[&Region],
    pieces: &[String],
) -> Result<(), (usize, String)> {
    let same = marker::parse(text).is_ok_and(|file| {
        let back: Vec<String> = file
            .segments
            .iter()
            .filter_map(|s| match s {
                Segment::Region(r) => Some(raw_lines(r)),
                Segment::Prose(_) => None,
            })
            .collect();
        back == pieces
    });
    if same {
        return Ok(());
    }
    let culprit = regions
        .iter()
        .zip(pieces)
        .find(|(r, p)| raw_lines(r) != **p)
        .map_or(regions[0], |(r, _)| *r);
    let fence = marker::unclosed_fences(original)
        .into_iter()
        .rfind(|&l| l < culprit.line);
    Err(match fence {
        Some(l) => (
            l,
            format!(
                "this fence is never closed, and the region at line {} would close it and hide the markers below; close the fence",
                culprit.line
            ),
        ),
        None => (
            culprit.line,
            "the rendered region would not parse back as a region".to_string(),
        ),
    })
}

fn clean(region: &Region, state: State) -> (String, RegionReport) {
    if region.sums.is_none() && region.body.is_empty() {
        return (
            raw_lines(region),
            report(region, state, Some(Action::Fresh), None),
        );
    }
    let opener = region
        .raw_opener
        .trim_end_matches(['\n', '\r'])
        .trim_start_matches([' ', '\t']);
    let text = region_text(region, opener, "", &marker::rendered_closer(None));
    (text, report(region, state, Some(Action::Cleaned), None))
}

/// Renders one region under `run`. `force` renders a fresh region too, so a
/// loader whose output moved without its inputs can be caught up.
fn render(
    region: &Region,
    state: State,
    force: bool,
    trusted: bool,
    loaders: &mut dyn Loaders,
) -> (String, RegionReport) {
    let kept = |action, stderr| {
        (
            raw_lines(region),
            report(region, state, Some(action), stderr),
        )
    };
    if state == State::Fresh && !force {
        return kept(Action::Fresh, None);
    }
    if region.opener.loader == "exec" && !trusted {
        return kept(Action::Untrusted, None);
    }
    let loaded = match loaders.load(region) {
        Ok(l) => l,
        Err(LoadError::Failed { stderr }) => return kept(Action::Failed, Some(stderr)),
        Err(LoadError::Hard(message)) => return kept(Action::Error, Some(message)),
    };
    let body = match sink::body(
        region.opener.sink,
        &region.opener.lang,
        loaded.text.as_bytes(),
    ) {
        Ok(b) => shape(region, &b),
        Err(message) => return kept(Action::Failed, Some(message)),
    };
    let constant = loader::format_constant(&region.opener.loader);
    let sums = Sums {
        input: sum::input(&region.opener, &region.indent, constant, &loaded.snapshot),
        output: sum::output(&body),
    };
    let text = region_text(
        region,
        &marker::rendered_opener(&region.opener),
        &body,
        &marker::rendered_closer(Some(&sums)),
    );
    (text, report(region, state, Some(Action::Written), None))
}
