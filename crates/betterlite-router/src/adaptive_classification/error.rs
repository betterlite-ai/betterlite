use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdaptiveClassificationError {
    #[error("the pinned classifier pattern at index {index} does not compile: {source}")]
    ClassifierPattern {
        index: usize,
        #[source]
        source: regex::Error,
    },
    #[error("adaptive_router_config.weights.quality must be a number between 0 and 1, got {value}")]
    QualityWeight { value: f64 },
    #[error("adaptive_router_config.weights.cost must be a number between 0 and 1, got {value}")]
    EconomyWeight { value: f64 },
    #[error(
        "adaptive_router_config.weights must sum to 1.0, got quality={quality} + cost={economy} = {sum}"
    )]
    WeightsSum {
        quality: f64,
        economy: f64,
        sum: f64,
    },
    #[error("model_info.adaptive_router_preferences.quality_tier must be 1, 2 or 3, got {value}")]
    QualityTier { value: i64 },
    #[error(
        "model_info.adaptive_router_preferences.strengths contains {value:?}, which is not a request type"
    )]
    UnknownStrength { value: String },
}
