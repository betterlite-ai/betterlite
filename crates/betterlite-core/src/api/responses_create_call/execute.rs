use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;

use crate::api::response_id_traversal::metadata::AffinityMetadata;
use crate::api::response_id_traversal::response_id::{rewrite_typed_response, transcode};
use crate::api::responses_create::bridge::Created;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_provider_selection::ParameterMap;

use super::error::{AsyncCreateCause, AsyncCreateError};
use super::prepare::PreparedCall;

// LiteLLM's isinstance(response, ResponsesAPIResponse) branch: encode the id with the deployment model_id, stamp the provider
pub(super) fn identity<Context>(
    created: Created<Context>,
    prepared: &PreparedCall,
) -> Result<Created<Context>, AsyncCreateError> {
    match created {
        Created::Responses(mut stamped) => {
            let affinity =
                affinity_metadata(&prepared.call.kwargs).map_err(|error| AsyncCreateError {
                    cause: Box::new(AsyncCreateCause::Create(error)),
                    provider: prepared.provider.clone(),
                    model: prepared.model.clone(),
                })?;
            rewrite_typed_response(&mut stamped.response, Some(&prepared.provider), &affinity)
                .map_err(|error| AsyncCreateError {
                    cause: Box::new(AsyncCreateCause::Create(CreateError::Rewrite(error))),
                    provider: prepared.provider.clone(),
                    model: prepared.model.clone(),
                })?;
            stamped.hidden.insert(
                String::from("custom_llm_provider"),
                Json::Text(prepared.provider.clone()),
            );
            Ok(Created::Responses(stamped))
        }
        other @ (Created::Raw(_) | Created::Live(_) | Created::Synthetic(_) | Created::Chat(_)) => {
            Ok(other)
        }
    }
}

// The update reads model_info.id and encrypted_content_affinity_enabled off litellm_metadata, which defaults to an empty dict
fn affinity_metadata(kwargs: &ParameterMap) -> Result<AffinityMetadata, CreateError> {
    match kwargs.get("litellm_metadata") {
        None | Some(JsonValue::Null) => Ok(AffinityMetadata::default()),
        Some(value @ JsonValue::Object(_)) => transcode(value).map_err(CreateError::from),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => Err(CreateError::Field {
            field: "litellm_metadata",
            expected: "an object",
        }),
    }
}
