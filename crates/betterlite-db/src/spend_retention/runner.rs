pub mod clock;
pub mod error;
pub mod groups;
pub mod job;
pub mod lifecycle;
pub mod lock;
pub mod partitions;
pub mod progress;

#[cfg(test)]
mod support;

// SPEND_LOG_CLEANUP_JOB_NAME: the Redis lease key the source acquires and releases around a run
pub const SPEND_LOG_CLEANUP_JOB_NAME: &str = "spend_log_cleanup";
