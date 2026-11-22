use std::sync::{Mutex, PoisonError};

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// update_environment_variables(model="", user=..., optional_params={"name": name}, litellm_params={"litellm_call_id": ..., "name": name}, custom_llm_provider=...)
pub(crate) struct LoggedCall<'call> {
    pub(crate) field: &'call str,
    pub(crate) value: &'call str,
    pub(crate) call_id: Option<&'call str>,
    pub(crate) provider: &'call str,
    pub(crate) call_type: &'call str,
    pub(crate) user: Option<String>,
}

pub(crate) fn log_call(
    logging: &Mutex<LoggingState>,
    call: &LoggedCall<'_>,
) -> Result<(), LoggingStateError> {
    // request_params is the one primary argument, and the same map is the optional params and the logged litellm params
    let mut request_params = IndexMap::new();
    request_params.insert(call.field.to_owned(), Json::Text(call.value.to_owned()));
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.call_id
            .map_or(Json::Null, |call_id| Json::Text(call_id.to_owned())),
    );
    litellm_params.insert(call.field.to_owned(), Json::Text(call.value.to_owned()));
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(call.provider.to_owned()),
    );
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_environment_variables(
        &mut state,
        &litellm_params,
        CallUpdate {
            model: Some(String::new()),
            user: call.user.clone(),
            optional_params: request_params,
            additional_params,
        },
    )?;
    call.call_type.clone_into(&mut state.call_type);
    drop(state);
    Ok(())
}
