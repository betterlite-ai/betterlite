mod payload;
mod response;
mod settings;
mod support;
mod transport;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::akto::payload::AktoInputs;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.op.as_str() {
            "init" => response::init(input),
            "payload" => payload::build(input),
            "response" => response::parse(input),
            "unreachable" => response::unreachable(input),
            "send" => transport::send(input, false),
            "fire_and_forget" => transport::send(input, true),
            _ => Self::not_built("b3751"),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct Input {
    pub(super) op: String,
    pub(super) settings: SettingsInput,
    pub(super) env: BTreeMap<String, String>,
    pub(super) inputs: Option<InputsInput>,
    pub(super) request_data: BTreeMap<String, GuardrailValue>,
    pub(super) status_code: Option<u16>,
    pub(super) include_response: bool,
    pub(super) answer: AnswerInput,
    pub(super) guardrails: Option<bool>,
    pub(super) ingest_data: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct SettingsInput {
    pub(super) akto_base_url: Option<String>,
    pub(super) akto_api_key: Option<String>,
    pub(super) akto_account_id: Option<String>,
    pub(super) akto_vxlan_id: Option<String>,
    pub(super) unreachable_fallback: Option<String>,
    pub(super) guardrail_timeout: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct InputsInput {
    pub(super) texts: Vec<String>,
    pub(super) model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct AnswerInput {
    pub(super) status: Option<u16>,
    pub(super) body: Option<GuardrailValue>,
    pub(super) raw_body: Option<String>,
}

pub(super) fn akto_inputs(input: &Input) -> AktoInputs {
    let Some(inputs) = &input.inputs else {
        return AktoInputs {
            texts: vec!["hello".to_owned()],
            model: None,
            tools: None,
            tool_calls: None,
            structured_messages: None,
        };
    };
    AktoInputs {
        texts: inputs.texts.clone(),
        model: inputs.model.clone(),
        tools: None,
        tool_calls: None,
        structured_messages: None,
    }
}

pub(super) fn inputs_json(input: &Input) -> Value {
    let Some(inputs) = &input.inputs else {
        return json!({"texts": ["hello"]});
    };
    let mut object = serde_json::Map::new();
    object.insert("texts".to_owned(), json!(inputs.texts));
    if let Some(model) = &inputs.model {
        object.insert("model".to_owned(), Value::String(model.clone()));
    }
    Value::Object(object)
}
