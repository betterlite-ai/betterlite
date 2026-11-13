use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::qualifire::QualifireGuardrail;
use betterlite_guardrails::qualifire::convert::message_rows;
use betterlite_guardrails::qualifire::error::QualifireError;
use betterlite_guardrails::qualifire::init::QualifireArgs;
use betterlite_guardrails::qualifire::settings::SecretLookup;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    settings: Settings,
    messages: Vec<GuardrailMessage>,
    answer: UpstreamAnswer,
    tools: Option<Vec<GuardrailValue>>,
    response: Option<Response>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    api_key: Option<String>,
    evaluation_id: Option<String>,
    prompt_injections: Option<bool>,
    hallucinations_check: Option<bool>,
    grounding_check: Option<bool>,
    pii_check: Option<bool>,
    content_moderation_check: Option<bool>,
    tool_selection_quality_check: Option<bool>,
    assertions: Option<Vec<String>>,
    on_flagged: Option<String>,
    timeout: Option<f64>,
}

#[derive(serde::Deserialize)]
struct Response {
    choices: Vec<Choice>,
}

#[derive(serde::Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(serde::Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

struct NoSecrets;

impl SecretLookup for NoSecrets {
    fn secret(&self, _name: &str) -> Option<SecretString> {
        None
    }
}

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        let script = MockScript::try_from(std::slice::from_ref(&input.answer))
            .map_err(|error| failure("MockError", error.to_string()))?;
        let mock =
            MockProvider::start(script).map_err(|error| failure("MockError", error.to_string()))?;
        let guardrail = build(input, &mock.base_url)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("OSError", error.to_string()))?;
        let (inputs, request_data, input_type) = prepare(input);
        let result = runtime.block_on(guardrail.apply_guardrail(inputs, &request_data, input_type));
        let calls = mock
            .finish()
            .map_err(|error| failure("MockError", error.to_string()))?;
        let request = calls.into_iter().next().map(|call| {
            json!({
                "path": call.request.path,
                "body": call.request.body,
                "x_qualifire_api_key": call.request.headers.get("x-qualifire-api-key"),
                "content_type": call.request.headers.get("content-type"),
            })
        });
        let (status, detail) = match result {
            Ok(_) => (None, Value::Null),
            Err(QualifireError::Blocked { detail }) => (
                Some(400),
                serde_json::to_value(detail)
                    .map_err(|error| failure("TypeError", error.to_string()))?,
            ),
            Err(QualifireError::HttpStatus { status }) => (Some(status.as_u16()), Value::Null),
            Err(error) => return Err(failure("QualifireError", error.to_string())),
        };
        Ok(json!({ "request": request, "status": status, "detail": detail }))
    }
}

fn build(input: &Call, base: &str) -> Result<QualifireGuardrail, Box<HelperError>> {
    let settings = &input.settings;
    let args = QualifireArgs {
        api_key: settings.api_key.clone().map(SecretString::from),
        api_base: Some(base.to_owned()),
        evaluation_id: settings.evaluation_id.clone(),
        prompt_injections: settings.prompt_injections,
        hallucinations_check: settings.hallucinations_check,
        grounding_check: settings.grounding_check,
        pii_check: settings.pii_check,
        content_moderation_check: settings.content_moderation_check,
        tool_selection_quality_check: settings.tool_selection_quality_check,
        assertions: settings.assertions.clone(),
        on_flagged: settings.on_flagged.clone(),
        name: "qualifire".to_owned(),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
        timeout: settings.timeout,
    };
    let client = base_builder()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))?;
    QualifireGuardrail::new(&args, &NoSecrets, &BTreeMap::new(), client)
        .map_err(|error| failure("QualifireError", error.to_string()))
}

// the request path hands the guardrail the message texts and the messages, the response path the choice texts
fn prepare(
    input: &Call,
) -> (
    GuardrailApiInputs,
    BTreeMap<String, GuardrailValue>,
    GuardrailInputType,
) {
    let messages = message_rows(&input.messages)
        .into_iter()
        .map(GuardrailValue::Object)
        .collect();
    let mut request_data =
        BTreeMap::from([("messages".to_owned(), GuardrailValue::Array(messages))]);
    if let Some(tools) = &input.tools {
        request_data.insert("tools".to_owned(), GuardrailValue::Array(tools.clone()));
    }
    let mut inputs = GuardrailApiInputs::default();
    let Some(response) = &input.response else {
        let texts = input
            .messages
            .iter()
            .filter_map(|message| match message.extra.get("content") {
                Some(GuardrailValue::Text(text)) => Some(text.clone()),
                _ => None,
            })
            .collect();
        inputs.texts = Some(texts);
        inputs.structured_messages = Some(input.messages.clone());
        inputs.model = Some("gpt-4o".to_owned());
        request_data.insert(
            "model".to_owned(),
            GuardrailValue::Text("gpt-4o".to_owned()),
        );
        return (inputs, request_data, GuardrailInputType::Request);
    };
    let texts = response
        .choices
        .iter()
        .filter_map(|choice| choice.message.content.clone())
        .collect();
    inputs.texts = Some(texts);
    (inputs, request_data, GuardrailInputType::Response)
}

fn failure(class_name: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class_name.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
