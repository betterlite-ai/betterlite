use std::error::Error as StdError;

use crate::prometheus_helpers::bounded_series_tracker::TrackerError;
use crate::prometheus_label_projection::prometheus_label_factory::LabelFactoryError;
use crate::prometheus_label_projection::tag_wildcard_matching::TagPatternError;

#[derive(Debug, thiserror::Error)]
pub enum LatencyRecordingError<MetricError>
where
    MetricError: StdError + 'static,
{
    #[error(transparent)]
    Labels(#[from] LabelFactoryError),
    #[error(transparent)]
    Context(#[from] TagPatternError),
    #[error(transparent)]
    Series(#[from] TrackerError<MetricError>),
    #[error("the latency sample does not fit in a decimal second value")]
    Overflow,
}

#[derive(Debug, thiserror::Error)]
pub enum OverheadRecordingError {
    #[error(transparent)]
    Labels(#[from] LabelFactoryError),
    #[error("the overhead sample does not fit in a decimal second value")]
    Overflow,
}
