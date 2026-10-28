pub mod process;
pub mod runtime;

// B002 ConfigLoader supplies GC threshold values through runtime::gc_thresholds without direct environment reads.

#[cfg(test)]
mod tests;
