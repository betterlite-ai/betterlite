use betterlite_models::fireworks_catalog_metadata::resolve_resource_name;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::FireworksResponsesError;
use super::instructions::{developer_as_system, fold_instructions};
use crate::response_input::{ResponsesInput, transform_responses_request as openai_request};

// LiteLLM's transform_responses_api_request: fold the instruction items, then let the OpenAI Responses transform build the body.
pub fn transform_responses_request(
    model: &str,
    input: &ResponsesInput,
    params: &Object,
    model_info: Option<&Object>,
) -> Result<Object, FireworksResponsesError> {
    let (instructions, folded_input) = match params.get("instructions") {
        Some(Json::Text(text)) => fold_instructions(input, Some(text.as_str())),
        None | Some(Json::Null) => fold_instructions(input, None),
        Some(other) => (Some(other.clone()), developer_items_as_system(input)),
    };
    let folded = folded_params(params, instructions);
    openai_request(
        &resolve_resource_name(model),
        &folded_input,
        &folded,
        ProviderId::FireworksAi,
        model_info,
    )
    .map_err(FireworksResponsesError::from)
}

fn developer_items_as_system(input: &ResponsesInput) -> ResponsesInput {
    let ResponsesInput::Items(items) = input else {
        return input.clone();
    };
    ResponsesInput::Items(
        items
            .iter()
            .map(|item| ResponsesItem(developer_as_system(&item.0)))
            .collect(),
    )
}

fn folded_params(params: &Object, instructions: Option<Json>) -> Object {
    let mut folded = Object::new();
    for (key, value) in params {
        if key != "instructions" {
            folded.insert(key.clone(), value.clone());
        }
    }
    if let Some(value) = instructions {
        folded.insert(String::from("instructions"), value);
    }
    folded
}
