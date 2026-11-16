use std::error::Error as StdError;

use betterlite_models::decisions::Json;

use crate::passthrough_reconstruction::error::ConvertError;
use crate::stream_usage_assembly::wire::Usage;
use crate::streamed_response_assembly::records::Chunk;

// The provider's iterator answers one SSE event at a time; this crate owns the loop around it.
pub trait EventChunkConverter {
    type Failure: StdError + Send + Sync + 'static;

    fn convert_event(&mut self, event: &str) -> Result<Option<Chunk>, ConvertError<Self::Failure>>;
}

// The provider's config prices and counts a usage object the way its own streams do.
pub trait UsageCalculator {
    type Failure: StdError + Send + Sync + 'static;

    fn calculate_usage(
        &self,
        usage_object: &Json,
        speed: Option<&str>,
    ) -> Result<Usage, Self::Failure>;
}
