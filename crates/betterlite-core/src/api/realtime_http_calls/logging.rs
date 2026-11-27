use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// What litellm_logging_obj.update_from_kwargs receives: the call's kwargs, the resolved routing and the raw optional params.
pub struct PreCall<'call> {
    pub kwargs: &'call IndexMap<String, Json>,
    pub model: &'call str,
    pub provider: &'call str,
    pub api_base: &'call str,
    pub optional_params: IndexMap<String, Json>,
}

pub fn log_pre_call(
    logging: &mut LoggingState,
    call: PreCall<'_>,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        String::from("api_base"),
        Json::Text(call.api_base.to_owned()),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        String::from("custom_llm_provider"),
        Json::Text(call.provider.to_owned()),
    );
    update_from_kwargs(
        logging,
        call.kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(call.model.to_owned()),
            user: None,
            optional_params: call.optional_params,
            additional_params,
        },
    )
}
