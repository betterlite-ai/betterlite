pub mod cache;
pub mod gc_config;
pub mod process;
pub mod router;
pub mod unsupported;

// B002 ConfigLoader supplies GC threshold values through gc_config::gc_thresholds without direct environment reads.

mod tests;
