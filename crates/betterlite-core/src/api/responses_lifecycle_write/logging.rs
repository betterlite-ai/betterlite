use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::call::ResponsesLifecycleWriteCall;

// LiteLLM's pre-call update, run only after the provider and its config validate; the named arguments carry no metadata slots, so local_vars overlaid by kwargs is the kwargs slots themselves
pub(super) fn update_logging(
    state: &mut LoggingState,
    call: &ResponsesLifecycleWriteCall,
    response_id: &str,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut kwargs = IndexMap::new();
    insert_slot(&mut kwargs, "metadata", call.params.metadata.as_ref());
    insert_slot(
        &mut kwargs,
        "litellm_metadata",
        call.params.litellm_metadata.as_ref(),
    );
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        String::from("litellm_call_id"),
        call.params
            .litellm_call_id
            .as_ref()
            .map_or(Json::Null, |id| Json::Text(id.clone())),
    );
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        String::from("response_id"),
        Json::Text(response_id.to_owned()),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        String::from("custom_llm_provider"),
        Json::Text(provider.to_owned()),
    );
    update_from_kwargs(
        state,
        &kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}

fn insert_slot(kwargs: &mut IndexMap<String, Json>, name: &str, slot: Option<&Json>) {
    if let Some(value) = slot {
        kwargs.insert(name.to_owned(), value.clone());
    }
}
