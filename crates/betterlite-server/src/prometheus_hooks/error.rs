use std::convert::Infallible;

use betterlite_logging::end_user::EndUserIdError;
use betterlite_logging::logging_metadata::tags::TagError;
use betterlite_logging::prometheus::bounded_series::CounterEmissionError;
use betterlite_logging::prometheus_label_projection::combined_custom_metadata::CombinedMetadataError;
use betterlite_logging::prometheus_label_projection::tag_wildcard_matching::TagPatternError;
use betterlite_logging::prometheus_latency::error::LatencyRecordingError;
use betterlite_logging::prometheus_rate_limits::error::PrometheusRateLimitError;
use betterlite_logging::prometheus_usage::error::PrometheusUsageError;

// the success hook stops at the first emitter that fails, so every emitter error is one variant here
#[derive(Debug, thiserror::Error)]
pub enum PrometheusHookError {
    #[error("standard_logging_object is required, got=None")]
    MissingPayload,
    #[error(transparent)]
    EndUser(#[from] EndUserIdError),
    #[error(transparent)]
    Tags(#[from] TagError),
    #[error(transparent)]
    Labels(#[from] TagPatternError),
    #[error(transparent)]
    CustomMetadata(#[from] CombinedMetadataError),
    #[error(transparent)]
    Counter(#[from] CounterEmissionError<Infallible>),
    #[error(transparent)]
    Usage(#[from] PrometheusUsageError<Infallible>),
    #[error(transparent)]
    RateLimit(#[from] PrometheusRateLimitError<Infallible>),
    #[error(transparent)]
    Latency(#[from] LatencyRecordingError<Infallible>),
    #[error("{message}")]
    Budget { message: String },
    #[error("the caller's UserAPIKeyAuth could not be read: {0}")]
    Principal(#[from] serde_json::Error),
}
