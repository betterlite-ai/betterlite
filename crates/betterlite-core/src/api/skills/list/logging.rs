use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_logging::logging_state::updates::update_from_kwargs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::skills::get_delete::call::LITELLM_PROXY_PROVIDER;

use super::call::ListSkillsCall;

// LiteLLM logs the transformed query dict, where limit is still an integer and page and source are strings
pub fn log_pre_call(
    logging: &mut LoggingState,
    call: &ListSkillsCall<'_>,
    provider: &str,
    query_params: &[(String, String)],
) -> Result<(), LoggingStateError> {
    let mut optional_params = IndexMap::new();
    for (name, value) in query_params {
        let logged = match (name.as_str(), value.parse::<i64>()) {
            ("limit", Ok(limit)) => Json::Number(Number::from(limit)),
            _ => Json::Text(value.clone()),
        };
        optional_params.insert(name.clone(), logged);
    }
    log_update(logging, call.kwargs, call, provider, optional_params)
}

// LiteLLM's hosted transformation handler logs the resolved pagination before the database call
pub fn log_hosted_pre_call(
    logging: &mut LoggingState,
    call: &ListSkillsCall<'_>,
    limit: i64,
    offset: i64,
) -> Result<(), LoggingStateError> {
    let mut optional_params = IndexMap::new();
    optional_params.insert("limit".to_owned(), Json::Number(Number::from(limit)));
    optional_params.insert("offset".to_owned(), Json::Number(Number::from(offset)));
    // list_skills passes no kwargs to the hosted handler, so its log line never copies caller metadata
    let kwargs = IndexMap::new();
    log_update(
        logging,
        &kwargs,
        call,
        LITELLM_PROXY_PROVIDER,
        optional_params,
    )
}

fn log_update(
    logging: &mut LoggingState,
    kwargs: &IndexMap<String, Json>,
    call: &ListSkillsCall<'_>,
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
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}
