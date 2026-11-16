pub mod coordinator;
pub mod manager;
pub mod ports;

mod analysis;
mod capabilities;
mod error;
mod heartbeat;
mod phases;
mod process;
mod report;
mod run;
mod status;

pub(crate) const PR_CONCURRENCY: usize = 3;

include!("roi_sync/tests/declarations.rs");
