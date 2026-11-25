#[path = "fixtures/answers.rs"]
mod answers;
#[path = "fixtures/config.rs"]
mod config;
#[path = "fixtures/dispatch.rs"]
mod dispatch;
#[path = "fixtures/events.rs"]
mod events;
#[path = "fixtures/logging.rs"]
mod logging;
#[path = "fixtures/ports.rs"]
mod ports;
#[path = "fixtures/rows.rs"]
mod rows;
#[path = "fixtures/tests.rs"]
mod tests;
pub(super) use events::provider_chunk;

pub(super) use answers::{requests, start_mock};
pub(super) use config::Configs;
pub(super) use dispatch::dispatch;
pub(super) use logging::LoggingProbe;
pub(super) use ports::{DispatchResolver, NoBridge, NoInterception, Nothing, Outbound, outbound};
pub(super) use rows::{ChunkKind, QuotaKind, ScriptedFailure};

#[path = "fixtures/errors.rs"]
mod errors;
pub(super) use errors::{AdapterError, DispatchFailure, FixtureError};
