use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_logging::logging_state::updates::update_from_kwargs;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoggingUpdateError {
    #[error("the call kwargs cannot be logged: {0}")]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    State(#[from] LoggingStateError),
}

// the parts of the resolved call that reach the logging object before the request goes out
pub struct CallLog<'call> {
    pub model: &'call str,
    pub config_dict: &'call Object,
    pub call_id: Option<&'call str>,
    pub provider: &'call str,
}

// _mark_async_entrypoint: LiteLLM aliases litellm_params and the model_call_details slot, so both copies the marker
pub fn mark_async_entrypoint(logging: Option<&mut LoggingState>, marker: &str, is_async: bool) {
    let Some(state) = logging else {
        return;
    };
    let existing = state.model_call_details.get("litellm_params");
    let mut params = match existing {
        Some(Json::Object(params)) => params.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            IndexMap::new()
        }
    };
    params.insert(marker.to_owned(), Json::Bool(is_async));
    state
        .litellm_params
        .insert(marker.to_owned(), Json::Bool(is_async));
    state
        .model_call_details
        .insert("litellm_params".to_owned(), Json::Object(params));
}

// Logging.update_from_kwargs: optional params, the resolved model and the call identity reach model_call_details
pub fn log_pre_call(
    logging: &mut LoggingState,
    kwargs: &Object,
    call: &CallLog<'_>,
) -> Result<(), LoggingUpdateError> {
    let logged_kwargs: IndexMap<String, Json> =
        serde_json::from_str(&serde_json::to_string(kwargs)?)?;
    let optional_params: IndexMap<String, Json> =
        serde_json::from_str(&serde_json::to_string(call.config_dict)?)?;
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.call_id
            .map_or(Json::Null, |call_id| Json::Text(call_id.to_owned())),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(call.provider.to_owned()),
    );
    update_from_kwargs(
        logging,
        &logged_kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(call.model.to_owned()),
            user: None,
            optional_params,
            additional_params,
        },
    )?;
    Ok(())
}
