pub mod agentops;
pub mod arize_callback;
pub mod levo;
pub mod logfire;
pub mod phoenix_callback;
pub mod traceloop;
pub mod weave;

mod tests;

// injected so a callback's wire payload never reads the process clock or a real UUID generator
pub type IdGenerator = fn() -> String;
