use crate::complexity_scoring::error::ScoringError;

#[derive(Debug, thiserror::Error)]
pub enum QualityRouterError {
    #[error(
        "model '{model}' is listed in available_models but has no model_info.litellm_routing_preferences"
    )]
    MissingPreferences { model: String },
    #[error("available_models are not present in the model list: {models:?}")]
    MissingModels { models: Vec<String> },
    #[error("complexity tier '{tier}' is not present in complexity_to_quality")]
    MissingComplexityMapping { tier: String },
    #[error("no model available for quality tier {tier} and no default_model configured")]
    NoModelForTier { tier: i64 },
    #[error("no user message and no default_model configured")]
    NoUserMessage,
    #[error("the text part at index {part} of message role '{role}' is not a string")]
    PartTextNotString { role: String, part: usize },
    #[error("the quality routing decision score is not finite")]
    NonFiniteScore,
    #[error("the routing decision is not JSON: {source}")]
    Wire {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Scoring(#[from] ScoringError),
}
