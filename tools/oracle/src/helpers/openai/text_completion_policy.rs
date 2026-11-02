use crate::helpers::{HelperError, HelperResult};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::text_completion_policy::reply::to_chat_model_response;
use betterlite_openai::text_completion_policy::{
    SUPPORTED_PARAMS, TextCompletionSettings, select_completion_provider,
    transform_completion_request,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

mod tests;

type Object = Map<String, Value>;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigValuesInput {
    settings: Object,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AllowlistInput {
    model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    model: String,
    messages: Vec<Value>,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    response_object: Option<Value>,
    model_response_object: Option<Object>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectionInput {
    provider: ProviderId,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TextCompletionCall {
    TextConfigValues(ConfigValuesInput),
    TextParameterAllowlist(AllowlistInput),
    SinglePromptTokens(RequestInput),
    MultiplePromptStrings(RequestInput),
    TextReplyAsChat(ReplyInput),
    TextConfigSelection(SelectionInput),
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn config_values(input: &ConfigValuesInput) -> Result<Value, Box<HelperError>> {
    let settings: TextCompletionSettings =
        serde_json::from_value(Value::Object(input.settings.clone()))
            .map_err(|error| invalid_input(error.to_string()))?;
    let config = provider_config(&settings).map_err(|error| invalid_input(error.to_string()))?;
    Ok(Value::Object(config))
}

fn request(input: &RequestInput) -> Result<Value, Box<HelperError>> {
    transform_completion_request(&input.model, &input.messages, input.optional_params.clone())
        .map(Value::Object)
        .map_err(|error| invalid_input(error.to_string()))
}

fn invoke(input: &TextCompletionCall) -> Result<Value, Box<HelperError>> {
    match input {
        TextCompletionCall::TextConfigValues(value) => config_values(value),
        TextCompletionCall::TextParameterAllowlist(_) => Ok(json!(SUPPORTED_PARAMS)),
        TextCompletionCall::SinglePromptTokens(value)
        | TextCompletionCall::MultiplePromptStrings(value) => request(value),
        TextCompletionCall::TextReplyAsChat(value) => to_chat_model_response(
            value.response_object.clone(),
            value.model_response_object.clone(),
            &uuid::Uuid::new_v4,
        )
        .map(Value::Object)
        .map_err(|error| invalid_input(error.to_string())),
        TextCompletionCall::TextConfigSelection(value) => {
            Ok(json!(select_completion_provider(value.provider)))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = TextCompletionCall;

    fn run(input: &Self::Input) -> HelperResult {
        invoke(input)
    }
}
