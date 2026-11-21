use std::sync::{Mutex, PoisonError};

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// one create or list call's update_from_kwargs arguments, the call type applied after it
pub(crate) struct LoggedCall<'call> {
    pub(crate) kwargs: &'call IndexMap<String, Json>,
    pub(crate) optional_params: &'call IndexMap<String, Json>,
    pub(crate) call_id: Option<&'call str>,
    pub(crate) provider: &'call str,
    pub(crate) call_type: &'call str,
}

// update_from_kwargs(kwargs, model="", optional_params=..., litellm_params={"litellm_call_id": ..., **params}, custom_llm_provider=...)
pub(crate) fn log_call(
    logging: &Mutex<LoggingState>,
    call: &LoggedCall<'_>,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.call_id
            .map_or(Json::Null, |call_id| Json::Text(call_id.to_owned())),
    );
    for (name, value) in call.optional_params {
        litellm_params.insert(name.clone(), value.clone());
    }
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(call.provider.to_owned()),
    );
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        call.kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(String::new()),
            user: None,
            optional_params: call.optional_params.clone(),
            additional_params,
        },
    )?;
    call.call_type.clone_into(&mut state.call_type);
    drop(state);
    Ok(())
}
