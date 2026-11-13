mod wire;

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::akto_mirroring::UnreachableFallback;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::akto::AktoMirroring;
use betterlite_guardrails::akto::error::AktoError;
use betterlite_guardrails::akto::payload::AktoInputs;
use betterlite_guardrails::akto::transport::AktoAnswer;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.op.as_str() {
            "init" => {
                let guardrail = settings(input)?;
                Ok(json!({
                    "base_url": guardrail.base_url,
                    "account_id": guardrail.account_id,
                    "vxlan_id": guardrail.vxlan_id,
                    "unreachable_fallback": fallback_name(guardrail.unreachable_fallback),
                    "timeout": guardrail.timeout.as_secs(),
                }))
            }
            "payload" => wire::build_payload(input),
            "response" => parse_response(input),
            "unreachable" => unreachable(input),
            "send" => wire::send(input, false),
            "fire_and_forget" => wire::send(input, true),
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

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct InputsInput {
    pub(super) texts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct AnswerInput {
    pub(super) status: Option<u16>,
    pub(super) body: Option<GuardrailValue>,
    pub(super) raw_body: Option<String>,
}

pub(super) fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
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
    json!(inputs)
}

fn settings(input: &Input) -> Result<AktoMirroring, Box<HelperError>> {
    let loader = ConfigLoader {
        lookup: input.env.clone(),
    };
    let config = loader
        .load(&source(&input.settings))
        .map_err(|error| failure("ConfigError", None, error.to_string()))?;
    let configured = config
        .sections
        .akto_mirroring
        .guardrails
        .first()
        .ok_or_else(|| failure("ConfigError", None, "no akto guardrail entry".to_owned()))?
        .clone();
    AktoMirroring::new(configured, reqwest::Client::default())
        .map_err(|error| failure("AktoError", None, error.to_string()))
}

fn source(settings: &SettingsInput) -> String {
    let mut parameters = vec!["guardrail: akto".to_owned()];
    for (name, value) in [
        ("akto_base_url", settings.akto_base_url.as_deref()),
        ("akto_api_key", settings.akto_api_key.as_deref()),
        ("akto_account_id", settings.akto_account_id.as_deref()),
        ("akto_vxlan_id", settings.akto_vxlan_id.as_deref()),
        (
            "unreachable_fallback",
            settings.unreachable_fallback.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            parameters.push(format!("{name}: {value:?}"));
        }
    }
    if let Some(value) = settings.guardrail_timeout {
        parameters.push(format!("guardrail_timeout: {value}"));
    }
    format!(
        "guardrails:\n  - guardrail_name: akto\n    litellm_params:\n      {}\n",
        parameters.join("\n      ")
    )
}

fn fallback_name(fallback: UnreachableFallback) -> &'static str {
    match fallback {
        UnreachableFallback::FailClosed => "fail_closed",
        UnreachableFallback::FailOpen => "fail_open",
    }
}

fn parse_response(input: &Input) -> crate::helpers::HelperResult {
    let guardrail = settings(input)?;
    let status = StatusCode::from_u16(input.answer.status.unwrap_or(200))
        .map_err(|error| failure("AktoError", None, error.to_string()))?;
    let body = answer_body(input)?.into_bytes();
    let verdict = AktoMirroring::handle_guardrail_response(&AktoAnswer { status, body })
        .map_err(|error| failure("AktoError", None, error.to_string()))?;
    Ok(json!({"allowed": verdict.allowed, "reason": verdict.reason}))
}

fn answer_body(input: &Input) -> Result<String, Box<HelperError>> {
    if let Some(raw) = &input.answer.raw_body {
        return Ok(raw.clone());
    }
    match &input.answer.body {
        Some(body) => serde_json::to_string(body)
            .map_err(|error| failure("JsonError", None, error.to_string())),
        None => Ok(String::new()),
    }
}

fn unreachable(input: &Input) -> crate::helpers::HelperResult {
    let guardrail = settings(input)?;
    let error = malformed_response_error()?;
    match guardrail.handle_unreachable(akto_inputs(input), &error) {
        Ok(_inputs) => Ok(json!({"passthrough": inputs_json(input)})),
        Err(error) => Err(failure(
            "HTTPException",
            Some(error.status_code().as_u16()),
            error.to_string(),
        )),
    }
}

fn malformed_response_error() -> Result<AktoError, Box<HelperError>> {
    let source = serde_json::from_str::<GuardrailValue>("not json")
        .err()
        .ok_or_else(|| {
            failure(
                "AktoError",
                None,
                "the malformed response parsed".to_owned(),
            )
        })?;
    Ok(AktoError::ResponseJson { source })
}
