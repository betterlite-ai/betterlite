pub mod candidates;
pub mod config;
pub mod eligibility;
pub mod entry;
pub mod error;
pub mod host;
pub mod request;
pub mod selection;

mod tests;

// choose_cached_model's state: the live proxy every read goes through, injected so nothing here reaches a global
pub struct CacheAlternatives<Host> {
    pub host: Host,
}
