use std::error::Error as StdError;

use crate::logging_metadata::decode::DecodeError;
use crate::prometheus::bounded_series::CounterEmissionError;
use crate::prometheus_label_projection::prometheus_label_factory::LabelFactoryError;
use crate::prometheus_label_projection::tag_wildcard_matching::TagPatternError;
use crate::prometheus_latency::error::OverheadRecordingError;
use crate::prometheus_rate_limits::error::PrometheusRateLimitError;

// a remaining-requests or remaining-tokens header the declared type cannot hold as a number
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum HeaderValueError {
    #[error("the {field} header value must be a number")]
    NotANumber { field: String },
}

// prometheus_client refuses a keyword call whose names differ from the declared ones
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum SchemaError {
    #[error("the {metric} metric has no label {label}")]
    Unsupported { metric: &'static str, label: String },
    #[error("the {metric} metric needs a value for label {label}")]
    Missing { metric: &'static str, label: String },
}

#[derive(Debug, thiserror::Error)]
pub enum PrometheusDeploymentError<MetricError>
where
    MetricError: StdError + 'static,
{
    #[error(transparent)]
    Counter(#[from] CounterEmissionError<MetricError>),
    #[error(transparent)]
    Labels(#[from] LabelFactoryError),
    #[error(transparent)]
    Context(#[from] TagPatternError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Header(#[from] HeaderValueError),
    #[error(transparent)]
    Overhead(#[from] OverheadRecordingError),
    #[error(transparent)]
    RateLimit(#[from] PrometheusRateLimitError<MetricError>),
    #[error(transparent)]
    Schema(#[from] SchemaError),
    #[error("the latency or overhead sample does not fit in a decimal value")]
    Overflow,
}
