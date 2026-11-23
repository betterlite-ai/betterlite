use betterlite_context::non_default_params::Params;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::{DataResidency, infer_openai_data_residency};
use betterlite_openai::response_input::ResponsesInput;
use indexmap::IndexMap;

use super::call::CompactResponsesCall;
use super::execute::Resolved;
use crate::api::responses_json::{decision_json, prompt_decision};

pub(super) fn log_call(
    state: &mut LoggingState,
    call: &CompactResponsesCall,
    resolved: &Resolved,
    filtered: &Params,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    for (name, value) in filtered {
        litellm_params.insert(name.clone(), decision_json(value));
    }
    litellm_params.insert(
        String::from("litellm_call_id"),
        call.kwargs
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    litellm_params.insert(
        String::from("data_residency"),
        residency_json(infer_openai_data_residency(
            Some(resolved.provider.as_str()),
            resolved.api_base.as_deref(),
        )),
    );
    let optional_params = filtered
        .iter()
        .map(|(name, value)| (name.clone(), decision_json(value)))
        .collect();
    let additional_params = IndexMap::from([(
        String::from("custom_llm_provider"),
        Json::Text(resolved.provider.clone()),
    )]);
    update_from_kwargs(
        state,
        &locals(call, resolved),
        Some(&litellm_params),
        CallUpdate {
            model: Some(resolved.model.clone()),
            user: None,
            optional_params,
            additional_params,
        },
    )
}

// LiteLLM logs local_vars, which holds the named arguments and then the caller's kwargs
fn locals(call: &CompactResponsesCall, resolved: &Resolved) -> IndexMap<String, Json> {
    let mut values = call.kwargs.clone();
    values.insert(String::from("model"), Json::Text(resolved.model.clone()));
    values.insert(
        String::from("custom_llm_provider"),
        Json::Text(resolved.provider.clone()),
    );
    values.insert(String::from("input"), input_json(&call.input));
    if let Some(instructions) = &call.instructions {
        values.insert(
            String::from("instructions"),
            Json::Text(instructions.clone()),
        );
    }
    if let Some(previous) = &call.previous_response_id {
        values.insert(
            String::from("previous_response_id"),
            Json::Text(previous.clone()),
        );
    }
    values
}

fn input_json(input: &ResponsesInput) -> Json {
    match input {
        ResponsesInput::Text(text) => Json::Text(text.clone()),
        ResponsesInput::Items(items) => {
            Json::List(items.iter().map(|item| prompt_decision(&item.0)).collect())
        }
    }
}

fn residency_json(residency: Option<DataResidency>) -> Json {
    match residency {
        Some(DataResidency::Eu) => Json::Text(String::from("eu")),
        Some(DataResidency::Us) => Json::Text(String::from("us")),
        None => Json::Null,
    }
}
