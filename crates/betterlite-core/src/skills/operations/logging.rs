use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_logging::logging_state::updates::update_from_kwargs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::skills::get_delete::call::LITELLM_PROXY_PROVIDER;
use crate::skills::store::SkillPage;

// The arguments each hosted skill call records before it reaches the store
pub(super) enum SkillLog<'request> {
    Create(Option<&'request str>),
    List(SkillPage),
    Skill(&'request str),
}

pub(super) fn log_skill_call(
    logging: &mut LoggingState,
    entry: SkillLog<'_>,
    litellm_call_id: Option<&str>,
) -> Result<(), LoggingStateError> {
    let mut optional_params = IndexMap::new();
    match entry {
        SkillLog::Create(display_title) => {
            let title = display_title.map_or(Json::Null, |title| Json::Text(title.to_owned()));
            optional_params.insert("display_title".to_owned(), title);
        }
        SkillLog::List(page) => {
            let limit = Json::Number(Number::from(page.limit));
            optional_params.insert("limit".to_owned(), limit);
            let offset = Json::Number(Number::from(page.offset));
            optional_params.insert("offset".to_owned(), offset);
        }
        SkillLog::Skill(skill_id) => {
            optional_params.insert("skill_id".to_owned(), Json::Text(skill_id.to_owned()));
        }
    }
    log_update(logging, optional_params, litellm_call_id)
}

// LiteLLM's transformation handler updates the call context before it reaches the store
fn log_update(
    logging: &mut LoggingState,
    optional_params: IndexMap<String, Json>,
    litellm_call_id: Option<&str>,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        litellm_call_id.map_or(Json::Null, |call_id| Json::Text(call_id.to_owned())),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(LITELLM_PROXY_PROVIDER.to_owned()),
    );
    // skills/main.py passes no kwargs to the hosted transformation, so no caller metadata is copied
    let kwargs = IndexMap::new();
    update_from_kwargs(
        logging,
        &kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}
