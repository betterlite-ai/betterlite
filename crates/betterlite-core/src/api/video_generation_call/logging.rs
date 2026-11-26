use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{CALL_TYPE, LITELLM_CALL_ID};
use crate::api::videos_http_transport::video_config::Params;

// the arguments update_from_kwargs is handed, after the logging object and the async flag were popped
pub(crate) struct LoggedGeneration<'call> {
    pub(crate) kwargs: &'call IndexMap<String, Json>,
    pub(crate) model: &'call str,
    pub(crate) user: Option<&'call str>,
    pub(crate) params: &'call Params,
    pub(crate) provider: &'call str,
    pub(crate) call_id: Option<&'call str>,
}

// update_from_kwargs(kwargs, model=model, user=user, optional_params=params,
// litellm_params={"litellm_call_id", **params}, custom_llm_provider=provider), then the call type
pub(crate) fn log_call(
    state: &mut LoggingState,
    call: &LoggedGeneration<'_>,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = Params::new();
    litellm_params.insert(
        LITELLM_CALL_ID.to_owned(),
        call.call_id
            .map_or(Json::Null, |call_id| Json::Text(call_id.to_owned())),
    );
    for (name, value) in call.params {
        litellm_params.insert(name.clone(), value.clone());
    }
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
            model: Some(call.model.to_owned()),
            user: call.user.map(str::to_owned),
            optional_params: call.params.clone(),
            additional_params,
        },
    )?;
    CALL_TYPE.clone_into(&mut state.call_type);
    Ok(())
}
