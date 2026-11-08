use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;

use crate::case::UpstreamAnswer;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    PreCall(PreCall),
    PostCall(PostCall),
    BackendTimeout(PreCall),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreCall {
    pub(crate) api_key: String,
    pub(crate) guardrail_name: String,
    pub(crate) call_type: String,
    #[serde(default)]
    pub(crate) messages: Option<Vec<GuardrailMessage>>,
    #[serde(default)]
    pub(crate) prompt: Option<GuardrailValue>,
    #[serde(default)]
    pub(crate) tools: Option<GuardrailValue>,
    #[serde(default)]
    pub(crate) input_recipe: Option<String>,
    #[serde(default)]
    pub(crate) output_recipe: Option<String>,
    #[serde(default)]
    pub(crate) answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostCall {
    pub(crate) api_key: String,
    pub(crate) guardrail_name: String,
    pub(crate) kind: ResponseKind,
    #[serde(default)]
    pub(crate) messages: Option<Vec<GuardrailMessage>>,
    #[serde(default)]
    pub(crate) prompt: Option<GuardrailValue>,
    #[serde(default)]
    pub(crate) tools: Option<GuardrailValue>,
    #[serde(default)]
    pub(crate) choices: Vec<GuardrailValue>,
    #[serde(default)]
    pub(crate) input_recipe: Option<String>,
    #[serde(default)]
    pub(crate) output_recipe: Option<String>,
    #[serde(default)]
    pub(crate) answer: Option<UpstreamAnswer>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseKind {
    Model,
    TextCompletion,
}
