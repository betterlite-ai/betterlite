use betterlite_openai::api::batch_records::CreateBatchRequest;
use indexmap::IndexMap;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, ItemsBoundary};
use crate::api::batch_provider_create::BatchProviderCreateInput;
use crate::api::batch_provider_create::error::BatchProviderCreateError;

// The six typed fields LiteLLM always sets, plus output_expires_after when the caller sent one
pub(super) fn create_request(
    input: &BatchProviderCreateInput,
    limits: ApiLimits,
) -> Result<CreateBatchRequest, BatchProviderCreateError> {
    limits.check(ApiBoundariesInput::Items(ItemsBoundary {
        known_count: input.metadata.as_ref().map_or(0, IndexMap::len)
            + input.extra_headers.as_ref().map_or(0, IndexMap::len)
            + input.extra_body.as_ref().map_or(0, IndexMap::len),
    }))?;
    Ok(CreateBatchRequest {
        completion_window: Some(input.completion_window),
        endpoint: Some(input.endpoint),
        input_file_id: Some(input.input_file_id.clone()),
        metadata: input.metadata.clone(),
        output_expires_after: input.output_expires_after,
        extra_headers: input.extra_headers.clone(),
        extra_body: input.extra_body.clone(),
        timeout: None,
    })
}
