use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::CallSetupError;
use crate::call_setup::RequestMap;

pub(super) fn apply_environment(
    state: &mut LoggingState,
    request: &RequestMap,
) -> Result<(), CallSetupError> {
    let update = CallUpdate {
        model: Some(state.model.clone()),
        user: Some(String::new()),
        optional_params: IndexMap::new(),
        additional_params: stream_options(request),
    };
    let mut params = IndexMap::new();
    params.insert(String::from("api_base"), Json::Text(String::new()));
    if let Some(metadata) = request.get("metadata") {
        params.insert(String::from("metadata"), metadata.clone());
    }
    if let Some(Json::Object(litellm_metadata)) = request.get("litellm_metadata") {
        params.insert(
            String::from("litellm_metadata"),
            Json::Object(litellm_metadata.clone()),
        );
        if matches!(params.get("metadata"), None | Some(Json::Null)) {
            params.insert(
                String::from("metadata"),
                Json::Object(litellm_metadata.clone()),
            );
        }
    }
    update_environment_variables(state, &params, update)?;
    Ok(())
}

fn stream_options(request: &RequestMap) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    if let Some(options) = request.get("stream_options")
        && !matches!(options, Json::Null)
    {
        params.insert(String::from("stream_options"), options.clone());
    }
    params
}
