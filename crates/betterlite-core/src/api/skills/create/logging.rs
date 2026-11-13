use betterlite_anthropic::skills::wire::CreateSkillRequest;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_logging::logging_state::updates::update_from_kwargs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::call::CreateSkillCall;

// LiteLLM's update_from_kwargs(model=None, optional_params=..., litellm_params={"litellm_call_id": ...}, custom_llm_provider=...)
pub fn log_pre_call(
    logging: &mut LoggingState,
    call: &CreateSkillCall<'_>,
    provider: &str,
    optional_params: IndexMap<String, Json>,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.litellm_call_id
            .map_or(Json::Null, |call_id| Json::Text(call_id.to_owned())),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    update_from_kwargs(
        logging,
        call.kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}

// create_skill logs the transformed request body, whose null fields the transform already dropped
pub(super) fn body_params(body: &CreateSkillRequest) -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    if let Some(display_title) = &body.display_title {
        params.insert(
            "display_title".to_owned(),
            Json::Text(display_title.clone()),
        );
    }
    if let Some(files) = &body.files {
        params.insert("files".to_owned(), Json::List(files.clone()));
    }
    for (name, value) in &body.extra {
        params.insert(name.clone(), value.clone());
    }
    params
}
