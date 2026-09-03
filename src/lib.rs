//! `computed` keeps marked regions of a markdown file current by computation.
//!
//! The modules follow the spec in `docs/spec/computed-v0.md`: `marker` parses
//! and serialises a file, `sink` shapes loader text, `render` decides what
//! every region becomes behind the `Loaders` seam, `loader` produces text and
//! snapshots, `fs` walks and writes, `trust` keeps the per-clone grants,
//! `report` prints, and `cli` ties them to five commands.

pub mod cli;
pub mod fs;
pub mod loader;
pub mod marker;
pub mod render;
pub mod report;
pub mod sink;
pub mod trust;
