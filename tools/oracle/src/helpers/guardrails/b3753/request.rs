use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;
use betterlite_guardrails::guardrails_ai_guardrail::InputFormat;

use crate::case::UpstreamAnswer;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AporiaModerationRequest {
    pub(crate) api_key: String,
    pub(crate) messages: Option<Vec<GuardrailMessage>>,
    pub(crate) dynamic_params: BTreeMap<String, GuardrailValue>,
    pub(crate) answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AporiaPostCallRequest {
    pub(crate) api_key: String,
    pub(crate) messages: Vec<GuardrailMessage>,
    pub(crate) response_string: Option<String>,
    pub(crate) dynamic_params: BTreeMap<String, GuardrailValue>,
    pub(crate) answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuardrailsAiConstructRequest {
    pub(crate) guard_name: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuardrailsAiProcessRequest {
    pub(crate) guard_name: String,
    pub(crate) input_format: InputFormat,
    pub(crate) last_user_message: Option<String>,
    pub(crate) dynamic_params: BTreeMap<String, GuardrailValue>,
    pub(crate) answer: Option<UpstreamAnswer>,
}
