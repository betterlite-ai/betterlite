use betterlite_models::decisions::Json;
use betterlite_openai::response_events::{ReplyBody, ResponsesReply};

use crate::api::response_id_traversal::metadata::AffinityMetadata;
use crate::api::response_id_traversal::response_id::rewrite_typed_response;

use super::call::CompactResponsesCall;
use super::error::CompactResponsesError;

pub(super) fn affinity_metadata(
    value: Option<&Json>,
) -> Result<AffinityMetadata, CompactResponsesError> {
    let Some(given) = value else {
        return Ok(AffinityMetadata::default());
    };
    match given {
        Json::Null => Ok(AffinityMetadata::default()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            let encoded = serde_json::to_value(given).map_err(CompactResponsesError::Metadata)?;
            serde_json::from_value(encoded).map_err(CompactResponsesError::Metadata)
        }
    }
}

// The reply's ID takes the provider and deployment the caller passed in litellm_metadata
pub(super) fn encode_response(
    reply: &mut ResponsesReply,
    call: &CompactResponsesCall,
    provider: &str,
) -> Result<(), CompactResponsesError> {
    let ReplyBody::Typed(response) = &mut reply.response else {
        return Ok(());
    };
    let metadata = affinity_metadata(call.kwargs.get("litellm_metadata"))?;
    rewrite_typed_response(response.as_mut(), Some(provider), &metadata)?;
    Ok(())
}
