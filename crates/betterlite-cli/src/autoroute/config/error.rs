use thiserror::Error;

use super::tier::Tier;

#[derive(Debug, Eq, Error, PartialEq)]
pub enum ConfigError {
    #[error("Tier {tier} references unknown chat model '{model}'")]
    TierModelNotDiscovered { tier: Tier, model: String },
    #[error("default_model '{model}' is not a known chat model")]
    DefaultModelNotDiscovered { model: String },
    #[error("classifier model '{model}' is not a known chat model")]
    ClassifierModelNotDiscovered { model: String },
    #[error("embedding model '{model}' is not a known embedding model")]
    EmbeddingModelNotDiscovered { model: String },
}
