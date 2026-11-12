use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::helpers::{HelperError, HelperResult};

mod choices;
mod endpoint;
mod reply;
mod stream;
#[path = "chat_choices/tests.rs"]
mod tests;

pub(crate) type Object = Map<String, Value>;

pub(crate) fn failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChoiceSteps {
    steps: Vec<ChoiceStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChoiceStep {
    #[serde(default)]
    optional_params: Option<Object>,
    #[serde(default)]
    json_mode: bool,
    choices: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkSteps {
    steps: Vec<ChunkStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkStep {
    chunk: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlSteps {
    steps: Vec<UrlStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    #[serde(default)]
    api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthSteps {
    steps: Vec<AuthStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthStep {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialCall {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    litellm_api_key: Option<String>,
    #[serde(default)]
    litellm_openai_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    litellm_api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    model: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ChatChoicesCall {
    ToolCallJsonContent(ChoiceSteps),
    ToolCallsFinishReason(ChoiceSteps),
    EmptyChoice(ChoiceSteps),
    ReasoningContentDelta(ChunkSteps),
    EmbeddedStreamError(ChunkSteps),
    ResponseHeaders(reply::ResponseSteps),
    ModelList(endpoint::ModelListSteps),
    QueryUrlJoin(UrlSteps),
    StreamEnvelopeFields(ChunkSteps),
    ChatAuthContentType(AuthSteps),
    ModelListDecodeFailure(endpoint::ModelListSteps),
    CredentialAccessorDefaults(CredentialCall),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = ChatChoicesCall;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            ChatChoicesCall::ToolCallJsonContent(steps)
            | ChatChoicesCall::ToolCallsFinishReason(steps)
            | ChatChoicesCall::EmptyChoice(steps) => choices::run(steps),
            ChatChoicesCall::ReasoningContentDelta(steps)
            | ChatChoicesCall::EmbeddedStreamError(steps)
            | ChatChoicesCall::StreamEnvelopeFields(steps) => Ok(stream::run(steps)),
            ChatChoicesCall::ResponseHeaders(steps) => reply::run(steps),
            ChatChoicesCall::ModelList(steps) | ChatChoicesCall::ModelListDecodeFailure(steps) => {
                endpoint::model_list(steps)
            }
            ChatChoicesCall::QueryUrlJoin(steps) => Ok(endpoint::url_join(steps)),
            ChatChoicesCall::ChatAuthContentType(steps) => Ok(endpoint::auth(steps)),
            ChatChoicesCall::CredentialAccessorDefaults(call) => endpoint::credentials(call),
        }
    }
}
