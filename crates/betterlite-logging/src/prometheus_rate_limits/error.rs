use std::error::Error as StdError;

use crate::prometheus_helpers::bounded_series_tracker::TrackerError;
use crate::prometheus_label_projection::combined_custom_metadata::CombinedMetadataError;
use crate::prometheus_label_projection::prometheus_label_factory::LabelFactoryError;

// an integer the caller declared, such as model_info.tpm or a legacy remaining variable, that the payload carries as another JSON type
#[derive(Debug, thiserror::Error)]
pub enum RateLimitValueError {
    #[error("the {field} value must be an integer")]
    NotAnInteger { field: String },
}

#[derive(Debug, thiserror::Error)]
pub enum PrometheusRateLimitError<MetricError>
where
    MetricError: StdError + 'static,
{
    #[error(transparent)]
    Value(#[from] RateLimitValueError),
    #[error(transparent)]
    Combined(#[from] CombinedMetadataError),
    #[error(transparent)]
    Labels(#[from] LabelFactoryError),
    #[error(transparent)]
    Series(#[from] TrackerError<MetricError>),
    #[error("the used rate limit does not fit in a decimal")]
    Overflow,
}
