use indexmap::IndexMap;

pub mod error;
pub mod family;
pub mod options;
pub mod parse;

mod tests;

pub type CallbackVars = IndexMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureFlags {
    pub otel_v2_enabled: bool,
}
