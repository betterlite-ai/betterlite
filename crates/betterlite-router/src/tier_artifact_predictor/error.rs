use crate::adaptive_classification::error::AdaptiveClassificationError;

#[derive(Debug, thiserror::Error)]
pub enum PredictorError {
    #[error("the tier artifact is {bytes} bytes, over the {limit}-byte parser bound")]
    ArtifactTooLarge { bytes: usize, limit: usize },
    #[error("the tier artifact could not be decoded: {source}")]
    ArtifactJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("unknown complexity router tier artifact: {name}")]
    UnknownArtifact { name: String },
    #[error("the tier artifact schema_version must be 1, got {version}")]
    SchemaVersion { version: u32 },
    #[error("the tier artifact declares tier {tier}, outside 1 through 4")]
    TierRange { tier: u32 },
    #[error("tier artifact statistic {field} must be finite")]
    NonFinite { field: String },
    #[error("tier artifact statistic {field} cannot be negative, got {value}")]
    NegativeSuccesses { field: String, value: f64 },
    #[error("tier artifact statistic {field} needs observations above zero, got {value}")]
    NonPositiveObservations { field: String, value: f64 },
    #[error(
        "tier artifact statistic {field} has {successes} successes over {observations} observations"
    )]
    SuccessesExceedObservations {
        field: String,
        successes: f64,
        observations: f64,
    },
    #[error("the tier artifact declares statistic {field} twice for tier {tier}")]
    DuplicateStatistic { field: String, tier: &'static str },
    #[error("the tier artifact must carry exactly one global statistic for each of the four tiers")]
    MissingGlobalStatistics,
    #[error("tier artifact request type {request_type} is not a known request type")]
    UnknownRequestType { request_type: String },
    #[error("tier artifact field {field} cannot be empty")]
    EmptyField { field: String },
    #[error("tier artifact dataset {index} declares no rows")]
    EmptyDatasetRows { index: usize },
    #[error("tier artifact routing_threshold must be between 0 and 1, got {threshold}")]
    ThresholdRange { threshold: f64 },
    #[error("tier artifact {field} must be above zero, got {mass}")]
    PriorMass { field: String, mass: f64 },
    #[error("the pinned cohort patterns do not compile: {source}")]
    Pattern {
        #[source]
        source: regex::Error,
    },
    #[error("the pinned prompt classifier patterns do not compile: {source}")]
    Classifier {
        #[source]
        source: AdaptiveClassificationError,
    },
}
