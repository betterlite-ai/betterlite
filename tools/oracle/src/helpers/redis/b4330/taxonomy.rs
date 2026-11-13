use betterlite_redis::batch::error::{BatchError, PipelineFailure};

pub(super) const fn classify(error: &BatchError) -> &'static str {
    match error {
        BatchError::Pipeline(failure) => match failure {
            PipelineFailure::Breaker(_) => "breaker",
            PipelineFailure::Client(_) | PipelineFailure::Live(_) => "connection",
            PipelineFailure::Encode { .. } | PipelineFailure::Ttl(_) => "pipeline",
        },
        BatchError::Reply(_)
        | BatchError::MissingReply { .. }
        | BatchError::MgetReply { .. }
        | BatchError::MgetValue { .. }
        | BatchError::IncrementReply { .. } => "reply",
        BatchError::MissingKeys { .. }
        | BatchError::Fallback(_)
        | BatchError::Delete(_)
        | BatchError::SetFallback(_)
        | BatchError::Script(_) => "connection",
        BatchError::Decode(_) => "decode",
        BatchError::Serialize { .. } => "pipeline",
        BatchError::Cancelled => "cancelled",
    }
}
