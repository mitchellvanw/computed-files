//! The two loaders, `tree` and `exec`, and the production `Loaders` adapter.

/// What every loader produces: the text a sink shapes, and the snapshot of
/// the inputs it read, which the input sum is taken over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub text: String,
    pub snapshot: Vec<u8>,
}

/// A loader error with its exit tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Tier 2: the tool could not answer. The file is skipped whole.
    Hard(String),
    /// Tier 1: the loader ran and failed. The previous body is kept.
    Failed { stderr: String },
}

/// The per-loader format constant folded into the input sum. Bumped by hand
/// only when that loader's output for the same inputs changes; a change to a
/// normalisation rule bumps both.
pub fn format_constant(loader: &str) -> u32 {
    match loader {
        "tree" => 1,
        "exec" => 1,
        other => panic!("unknown loader {other:?} reached the format table"),
    }
}
