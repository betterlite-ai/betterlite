#![cfg(test)]

#[path = "tests/deferred_log.rs"]
pub(crate) mod deferred_log;
#[path = "tests/memory_only.rs"]
mod memory_only;
#[path = "tests/redis_branches.rs"]
mod redis_branches;
#[path = "tests/redis_ops.rs"]
mod redis_ops;
#[path = "tests/support.rs"]
pub(crate) mod support;
