use std::sync::{Mutex, PoisonError};

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::container_file_list::{CALL_TYPE, ContainerFileListRequest};

// update_from_kwargs(kwargs, model="", optional_params={"container_id", "after", "limit", "order"}, litellm_params={"litellm_call_id": ...}, custom_llm_provider=...)
pub(super) fn log_call(
    logging: &Mutex<LoggingState>,
    request: &ContainerFileListRequest<'_>,
    kwargs: &IndexMap<String, Json>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        request
            .kwargs
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(String::new()),
            user: None,
            optional_params: logged_optional_params(request),
            additional_params,
        },
    )?;
    CALL_TYPE.clone_into(&mut state.call_type);
    drop(state);
    Ok(())
}

// LiteLLM logs all four pagination names, with None for the ones the caller left out
fn logged_optional_params(request: &ContainerFileListRequest<'_>) -> IndexMap<String, Json> {
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "container_id".to_owned(),
        Json::Text(request.container_id.to_owned()),
    );
    optional_params.insert(
        "after".to_owned(),
        request
            .after
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    optional_params.insert(
        "limit".to_owned(),
        request
            .limit
            .map_or(Json::Null, |number| Json::Number(Number::from(number))),
    );
    optional_params.insert(
        "order".to_owned(),
        request
            .order
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    optional_params
}
