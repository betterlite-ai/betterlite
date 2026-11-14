pub mod context;
pub mod error;
pub mod hook;
mod logging;
pub mod merge;
pub mod plan;
pub mod state;
mod text;
pub mod trim;

pub const STREAM_HOLDBACK_CHARS: usize = 50;
pub const STREAM_SCAN_CONTEXT_CHARS: usize = 512;
