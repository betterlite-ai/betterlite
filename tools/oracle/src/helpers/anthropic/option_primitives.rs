use betterlite_anthropic::option_primitives::context_management::map_context_management;
use betterlite_anthropic::option_primitives::user_id::UserIdPolicy;
use betterlite_anthropic::option_primitives::{
    CUSTOM_LLM_PROVIDER, OptionDefaults, WebSearchOptions, get_max_tokens_for_model,
    map_stop_sequences, map_web_search_tool, resolved_provider,
};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Defaults(DefaultsCase),
    ModelOutputCapacity(CapacityCase),
    StopSequenceFilter(StopCase),
    WebSearchContextSize(WebSearchCase),
    ContextCompactionEdit(ContextCase),
    UserIdentifierFilter(UserIdCase),
}

#[derive(Debug, Deserialize)]
struct DefaultsCase {
    defaults: OptionDefaults,
    model: Option<String>,
    default_chat_max_tokens: i64,
    configured_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CapacityCase {
    models: Vec<Option<String>>,
    default_chat_max_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct StopCase {
    drop_params: bool,
    stops: Vec<Json>,
}

#[derive(Debug, Deserialize)]
struct WebSearchCase {
    values: Vec<WebSearchOptions>,
}

#[derive(Debug, Deserialize)]
struct ContextCase {
    values: Vec<Json>,
}

#[derive(Debug, Deserialize)]
struct UserIdCase {
    ids: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum Answer {
    Value { value: Value },
    Raised { raised: bool },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Defaults(case) => defaults_answer(case),
            Input::ModelOutputCapacity(case) => Ok(capacity_answer(case)),
            Input::StopSequenceFilter(case) => Ok(stop_answers(case)),
            Input::WebSearchContextSize(case) => Ok(web_search_answers(case)),
            Input::ContextCompactionEdit(case) => Ok(context_answers(case)),
            Input::UserIdentifierFilter(case) => user_id_answer(case),
        }
    }
}

fn defaults_answer(case: &DefaultsCase) -> HelperResult {
    let config = case
        .defaults
        .get_config(
            case.model.as_deref(),
            &BundledCatalog::default().get(),
            case.default_chat_max_tokens,
        )
        .map_err(|error| {
            Box::new(HelperError {
                class: "OptionConfig".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    Ok(json!({
        "config": config,
        "custom_llm_provider": case
            .configured_provider
            .clone()
            .unwrap_or_else(|| CUSTOM_LLM_PROVIDER.to_owned()),
        "resolved_provider": resolved_provider(case.configured_provider.as_deref()),
    }))
}

fn capacity_answer(case: &CapacityCase) -> Value {
    let catalog = BundledCatalog::default().get();
    let values: Vec<i64> = case
        .models
        .iter()
        .map(|model| {
            get_max_tokens_for_model(&catalog, model.as_deref(), case.default_chat_max_tokens)
        })
        .collect();
    json!({ "values": values })
}

fn answered<T, E>(result: Result<T, E>) -> Answer
where
    T: serde::Serialize,
{
    match result {
        Ok(value) => Answer::Value {
            value: json!(value),
        },
        Err(_) => Answer::Raised { raised: true },
    }
}

fn stop_answers(case: &StopCase) -> Value {
    let answers: Vec<Answer> = case
        .stops
        .iter()
        .map(|stop| answered(map_stop_sequences(stop, case.drop_params)))
        .collect();
    json!({ "values": answers })
}

fn web_search_answers(case: &WebSearchCase) -> Value {
    let answers: Vec<Answer> = case
        .values
        .iter()
        .map(|options| answered(map_web_search_tool(options)))
        .collect();
    json!({ "values": answers })
}

fn context_answers(case: &ContextCase) -> Value {
    let answers: Vec<Answer> = case
        .values
        .iter()
        .map(|value| answered(map_context_management(value)))
        .collect();
    json!({ "values": answers })
}

fn user_id_answer(case: &UserIdCase) -> HelperResult {
    let Ok(policy) = UserIdPolicy::compile() else {
        return Err(Box::new(HelperError {
            class: "UserIdPolicy".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "a user id pattern does not compile".to_owned(),
        }));
    };
    let values: Vec<bool> = case
        .ids
        .iter()
        .map(|user_id| policy.accepts(user_id))
        .collect();
    Ok(json!({ "values": values }))
}
