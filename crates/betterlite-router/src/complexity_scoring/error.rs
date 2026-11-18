#[derive(Debug, thiserror::Error)]
pub enum ScoringError {
    #[error("scoring input {field} must be finite")]
    NonFinite { field: String },
    #[error("invalid scoring pattern: {source}")]
    Pattern {
        #[source]
        source: regex::Error,
    },
    #[error("the weighted complexity score is not finite")]
    NonFiniteScore,
    #[error("no model configured for tier {tier} and no default_model set")]
    NoTierModel { tier: String },
    #[error("tier {tier} is configured with a pool of several models")]
    TierPool { tier: String },
    #[error("the pinned scoring defaults are unreadable: {source}")]
    Defaults {
        #[source]
        source: serde_json::Error,
    },
    #[error("the routing decision record could not be serialized: {source}")]
    Record {
        #[source]
        source: serde_json::Error,
    },
}

impl From<regex::Error> for ScoringError {
    fn from(source: regex::Error) -> Self {
        Self::Pattern { source }
    }
}

impl From<serde_json::Error> for ScoringError {
    fn from(source: serde_json::Error) -> Self {
        Self::Defaults { source }
    }
}
