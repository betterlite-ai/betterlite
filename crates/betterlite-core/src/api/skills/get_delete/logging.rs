use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_logging::logging_state::updates::update_from_kwargs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::call::SkillCall;

// the matching LiteLLM call is litellm_logging_obj.update_from_kwargs(..., optional_params={"skill_id": skill_id})
pub fn log_pre_call(
    logging: &mut LoggingState,
    call: &SkillCall<'_>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut optional_params = IndexMap::new();
    optional_params.insert("skill_id".to_owned(), Json::Text(call.skill_id.to_owned()));
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
