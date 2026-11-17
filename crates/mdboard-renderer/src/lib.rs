//! Generated Markdown views for the 1.0 process board.

use std::collections::BTreeMap;
use std::path::PathBuf;

pub mod process;

/// The complete set of generated Markdown views for a board.
#[derive(Debug, Clone)]
pub struct RenderPlan {
    pub files: BTreeMap<PathBuf, String>,
}
