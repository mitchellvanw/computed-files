//! Decides what every region becomes. Pure: a parsed file, a mode, a trust
//! flag and a `Loaders` seam in; the new text and a report per region out.
//! Owns the sums, the freshness cache, the states, the refuse rule and its
//! per-file consequence, the untrusted skip, loader failure keeping the
//! body, and `clean`.

use std::fmt;

use crate::loader::{self, LoadError, Loaded};
use crate::marker::{self, File, Region, Segment, Sums};
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
    Cleaned,
    WouldClean,
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
            Action::Cleaned => "cleaned",
            Action::WouldClean => "would clean",
        })
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

    /// The exit tier this file contributes: 2 for an error, 1 when the
    /// content said no (a write, a refusal, a failure, an untrusted region,
    /// or drift under `check`), else 0.
    pub fn tier(&self) -> u8 {
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

    const DOMAIN: &str = "computed-in/1\n";

    fn hex16(hash: blake3::Hash) -> String {
        hash.to_hex()[..16].to_string()
    }

    pub fn input(opener: &Opener, format_constant: u32, snapshot: &[u8]) -> String {
        let mut h = blake3::Hasher::new();
        h.update(DOMAIN.as_bytes());
        h.update(format!("{}/{}\n", opener.loader, format_constant).as_bytes());
        h.update(opener.canonical().as_bytes());
        h.update(b"\n");
        h.update(snapshot);
        hex16(h.finalize())
    }

    pub fn output(body: &str) -> String {
        hex16(blake3::hash(body.as_bytes()))
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
            let input_ok = sum::input(&region.opener, constant, snapshot) == sums.input;
            match (input_ok, body_ok) {
                (true, true) => State::Fresh,
                (false, true) => State::Stale,
                (true, false) => State::Edited,
                (false, false) => State::StaleEdited,
            }
        }
    }
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

/// The closer's own terminator, so a file that ends without a newline stays so.
fn closer_terminator(region: &Region) -> &'static str {
    if region.raw_closer.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

fn region_text(region: &Region, opener_line: &str, body: &str, closer_line: &str) -> String {
    format!(
        "{indent}{opener_line}\n{body}{indent}{closer_line}{term}",
        indent = region.indent,
        term = closer_terminator(region)
    )
}

/// Renders one parsed file under `mode`. Performs no I/O.
pub fn file(parsed: &File, mode: Mode, trusted: bool, loaders: &mut dyn Loaders) -> Rendered {
    let regions: Vec<&Region> = parsed
        .segments
        .iter()
        .filter_map(|s| match s {
            Segment::Region(r) => Some(r),
            Segment::Prose(_) => None,
        })
        .collect();

    // Snapshots first: they always run, and a hard error skips the file whole.
    let mut states = Vec::with_capacity(regions.len());
    for region in &regions {
        if matches!(mode, Mode::Clean { .. }) {
            states.push(body_state(region));
            continue;
        }
        let snapshot = match loaders.snapshot(region) {
            Ok(s) => s,
            Err(LoadError::Hard(message)) => {
                return Rendered::Error {
                    line: region.line,
                    message,
                }
            }
            Err(LoadError::Failed { stderr }) => {
                return Rendered::Error {
                    line: region.line,
                    message: stderr,
                }
            }
        };
        states.push(state_of(region, snapshot.as_deref()));
    }

    if mode == Mode::Check {
        let reports = regions
            .iter()
            .zip(&states)
            .map(|(r, &s)| report(r, s, None, None))
            .collect();
        return Rendered::Unchanged { regions: reports };
    }

    // The hand-edit policy: one edited region refuses the whole file.
    if !mode.force() && states.iter().any(|s| s.edited()) {
        let reports = regions
            .iter()
            .zip(&states)
            .map(|(r, &s)| {
                let action = if s.edited() {
                    Action::Refused
                } else {
                    Action::Kept
                };
                report(r, s, Some(action), None)
            })
            .collect();
        return Rendered::Refused { regions: reports };
    }

    let mut text = String::new();
    let mut reports = Vec::with_capacity(regions.len());
    let mut states = states.into_iter();
    for segment in &parsed.segments {
        let region = match segment {
            Segment::Prose(p) => {
                text.push_str(p);
                continue;
            }
            Segment::Region(r) => r,
        };
        let state = states.next().expect("one state per region");
        let (piece, rep) = match mode {
            Mode::Clean { .. } => clean(region, state),
            Mode::Run { .. } | Mode::DryRun { .. } => render(region, state, trusted, loaders),
            Mode::Check => unreachable!("check returned above"),
        };
        text.push_str(&piece);
        reports.push(rep);
    }

    let original = marker::serialise(parsed);
    if text == original {
        for r in &mut reports {
            if matches!(
                r.action,
                Some(Action::Written | Action::WouldWrite | Action::Cleaned | Action::WouldClean)
            ) {
                r.action = Some(Action::Fresh);
            }
        }
        return Rendered::Unchanged { regions: reports };
    }
    let dry = matches!(
        mode,
        Mode::DryRun { .. } | Mode::Clean { dry_run: true, .. }
    );
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

fn render(
    region: &Region,
    state: State,
    trusted: bool,
    loaders: &mut dyn Loaders,
) -> (String, RegionReport) {
    if state == State::Fresh {
        return (
            raw_lines(region),
            report(region, state, Some(Action::Fresh), None),
        );
    }
    if region.opener.loader == "exec" && !trusted {
        return (
            raw_lines(region),
            report(region, state, Some(Action::Untrusted), None),
        );
    }
    let failed = |stderr: String| {
        (
            raw_lines(region),
            report(region, state, Some(Action::Failed), Some(stderr)),
        )
    };
    let loaded = match loaders.load(region) {
        Ok(l) => l,
        Err(LoadError::Failed { stderr }) => return failed(stderr),
        Err(LoadError::Hard(message)) => return failed(message),
    };
    let body = match sink::body(
        region.opener.sink,
        &region.opener.lang,
        loaded.text.as_bytes(),
    ) {
        Ok(b) => b,
        Err(message) => return failed(message),
    };
    let constant = loader::format_constant(&region.opener.loader);
    let sums = Sums {
        input: sum::input(&region.opener, constant, &loaded.snapshot),
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
