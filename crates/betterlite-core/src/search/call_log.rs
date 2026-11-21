use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_logging::logging_state::{LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_openai::search::records::SearchResponse;
use indexmap::IndexMap;

use crate::api::search_providers::parallel_ai::request::PARALLEL_AI_USAGE_PARAM;

pub(super) struct SearchLog<'call> {
    pub(super) kwargs: &'call IndexMap<String, Json>,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) complete_url: &'call str,
    pub(super) provider: &'call str,
}

pub(super) fn log_search(
    state: &mut LoggingState,
    call: SearchLog<'_>,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.kwargs
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    litellm_params.insert(
        "api_base".to_owned(),
        Json::Text(call.complete_url.to_owned()),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(call.provider.to_owned()),
    );
    update_from_kwargs(
        state,
        call.kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(format!("{}/search", call.provider)),
            user: None,
            optional_params: call.optional_params,
            additional_params,
        },
    )
}

// Parallel AI's reply hook writes its usage onto the logging object in LiteLLM; here it rides on the response
pub(super) fn record_provider_usage(
    state: &mut LoggingState,
    response: &SearchResponse,
) -> Result<(), serde_json::Error> {
    if let Some(usage) = response.hidden_params.get(PARALLEL_AI_USAGE_PARAM) {
        state.optional_params.insert(
            PARALLEL_AI_USAGE_PARAM.to_owned(),
            serde_json::from_value(usage.clone())?,
        );
    }
    Ok(())
}
