use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_logging::logging_state::{LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolvedModel;
use indexmap::IndexMap;

use super::call::CallCommon;

// update_from_kwargs receives the call's kwargs, the resolved routing, litellm_params' api_base and the raw optional params.
pub fn log_pre_call(
    logging: &mut LoggingState,
    common: &CallCommon<'_>,
    resolved: &ResolvedModel,
    api_base: &str,
    optional_params: IndexMap<String, Json>,
) -> Result<(), LoggingStateError> {
    let litellm_params = params([("api_base", Json::Text(api_base.to_owned()))]);
    let additional_params = params([(
        "custom_llm_provider",
        Json::Text(resolved.provider.slug().to_owned()),
    )]);
    update_from_kwargs(
        logging,
        common.kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(resolved.model.clone()),
            user: None,
            optional_params,
            additional_params,
        },
    )
}

// The maps update_from_kwargs reads, keyed as LiteLLM names them.
#[must_use]
pub fn params<const N: usize>(entries: [(&'static str, Json); N]) -> IndexMap<String, Json> {
    entries
        .into_iter()
        .map(|(name, value)| (String::from(name), value))
        .collect()
}
