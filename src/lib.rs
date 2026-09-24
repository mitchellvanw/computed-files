//! `computed` keeps marked regions of a markdown file current by computation.
//!
//! The modules follow the spec in `docs/spec/computed-v0.md`: `marker` parses
//! and serialises a file, `sink` shapes loader text, `render` decides what
//! every region becomes behind the `Loaders` seam, `loader` produces text and
//! snapshots, `fs` walks and writes, `trust` keeps the per-clone grants,
//! `report` prints, and `cli` ties them to five commands.

pub mod adopt;
pub mod affected;
pub mod allow;
pub mod cli;
pub mod config;
pub mod dupes;
pub mod fs;
pub mod git;
pub mod graph;
pub mod guard;
pub mod index;
pub mod loader;
pub mod lsp;
pub mod marker;
pub mod merge;
pub mod project;
pub mod remote;
pub mod render;
pub mod report;
pub mod sink;
pub mod stats;
pub mod survey;
pub mod symbol;
pub mod table;
pub mod toc;
pub mod transcript;
pub mod truncate;
pub mod trust;
pub mod update;
pub mod watch;
pub mod why;
