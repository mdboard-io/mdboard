//! Core data model for mdboard: actors, the process entities and cost types.

pub mod actor;
pub mod config;
pub mod error;
pub mod estimate;
pub mod process;
pub mod version;

pub use actor::{Actor, ActorKind, Actors};
pub use config::CostConfig;
pub use error::{Error, Result};
pub use estimate::EffortEstimate;
pub use version::Version;
