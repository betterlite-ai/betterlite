use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::Client;
use uuid::Uuid;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::hiddenlayer::evaluation::error::{EvaluationError, as_guardrail_message, blocked};
use crate::hiddenlayer::evaluation::metadata;
use crate::hiddenlayer::evaluation::text_part;
use crate::hiddenlayer::evaluation::wire;
use crate::hiddenlayer::evaluation::{
    BLOCK_MESSAGE, EvaluationArgs, REQUEST_EVALUATIONS_PATH, RESPONSE_EVALUATIONS_PATH,
    VIOLATED_POLICY, detection_headers, evaluation_timeout, guardrail_settings,
};
use crate::hiddenlayer::hiddenlayer_authentication::HiddenlayerAuth;
use crate::hiddenlayer::hiddenlayer_authentication::detection::{
    DetectionRequest, DetectionResponse,
};
use crate::hiddenlayer::hiddenlayer_authentication::token::JwtTransport;
use crate::primitives::json_value::JsonValue;

mod tests;

pub const VERSION: &str = "2";
const BLOCK_ACTION: &str = "block";

// LiteLLM's HiddenlayerGuardrailV2: the whole request or response payload is evaluated
pub struct HiddenlayerV2Guardrail {
    pub base: GuardrailBase,
    pub auth: HiddenlayerAuth,
    pub timeout: Duration,
    pub new_id: fn() -> Uuid,
}

impl HiddenlayerV2Guardrail {
    pub fn new<L: EnvironmentLookup>(
        args: &EvaluationArgs,
        environment: &L,
        client: Client,
        transport: Arc<dyn JwtTransport>,
    ) -> Result<Self, EvaluationError> {
        let auth = HiddenlayerAuth::new(&args.credentials(), environment, client, transport)?;
        let timeout = evaluation_timeout(args.timeout)?;
        let base = GuardrailBase::new(guardrail_settings(args, None), true)?;
        Ok(Self {
            base,
            auth,
            timeout,
            new_id: Uuid::new_v4,
        })
    }

    pub async fn evaluate(
        &self,
        mut inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, EvaluationError> {
        let headers = metadata::request_headers(request_data, logging_obj);
        let hl_headers = roundtrip_headers(&headers, request_data, self.new_id);
        let payload = match input_type {
            GuardrailInputType::Request => request_payload(&inputs)?,
            GuardrailInputType::Response => response_payload(&inputs)?,
        };
        let response = self
            .call_hiddenlayer(&payload, input_type, &hl_headers)
            .await?;
        let output: GuardrailValue = serde_json::from_slice(&response.body)
            .map_err(|source| EvaluationError::Decode { source })?;
        let action = metadata::header_value(&response.headers, metadata::RUNTIME_ACTION_HEADER, "");
        if action.eq_ignore_ascii_case(BLOCK_ACTION) {
            return Err(blocked(vec![
                ("error", GuardrailValue::Text(VIOLATED_POLICY.to_owned())),
                (
                    "hiddenlayer_guardrail_response",
                    GuardrailValue::Text(BLOCK_MESSAGE.to_owned()),
                ),
            ]));
        }
        match input_type {
            GuardrailInputType::Request => write_request(&mut inputs, &output),
            GuardrailInputType::Response => write_response(&mut inputs, &output),
        }?;
        Ok(inputs)
    }

    async fn call_hiddenlayer(
        &self,
        payload: &JsonValue,
        input_type: GuardrailInputType,
        hl_headers: &BTreeMap<String, String>,
    ) -> Result<DetectionResponse, EvaluationError> {
        let path = match input_type {
            GuardrailInputType::Request => REQUEST_EVALUATIONS_PATH,
            GuardrailInputType::Response => RESPONSE_EVALUATIONS_PATH,
        };
        let request = DetectionRequest {
            path,
            headers: detection_headers(VERSION, None, Some(hl_headers))?,
            body: payload,
        };
        self.auth
            .post_detection_with_timeout(&request, Some(self.timeout))
            .await
            .map_err(EvaluationError::from)
    }
}

// LiteLLM lowercases every hl- header, defaults the requester id and injects a roundtrip id
#[must_use]
pub fn roundtrip_headers(
    headers: &BTreeMap<String, String>,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    new_id: fn() -> Uuid,
) -> BTreeMap<String, String> {
    let mut hl_headers: BTreeMap<String, String> = headers
        .iter()
        .filter(|(name, _value)| name.starts_with("hl-"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if !hl_headers.contains_key(metadata::REQUESTER_HEADER) {
        hl_headers.insert(
            metadata::REQUESTER_HEADER.to_owned(),
            metadata::DEFAULT_REQUESTER.to_owned(),
        );
    }
    if !hl_headers.contains_key(metadata::ROUNDTRIP_HEADER) {
        let id = new_id().to_string();
        if let Some(GuardrailValue::Object(proxy)) = request_data.get_mut("proxy_server_request")
            && let Some(GuardrailValue::Object(headers)) = proxy.get_mut("headers")
        {
            headers.insert(
                metadata::ROUNDTRIP_HEADER.to_owned(),
                GuardrailValue::Text(id.clone()),
            );
            hl_headers.insert(metadata::ROUNDTRIP_HEADER.to_owned(), id);
        }
    }
    hl_headers
}

pub fn request_payload(inputs: &GuardrailApiInputs) -> Result<JsonValue, EvaluationError> {
    Ok(wire::object(vec![
        ("messages", wire::to_json(&inputs.structured_messages)?),
        ("model", wire::to_json(&inputs.model)?),
        ("tools", wire::to_json(&inputs.tools)?),
    ]))
}

pub fn response_payload(inputs: &GuardrailApiInputs) -> Result<JsonValue, EvaluationError> {
    if let Some(texts) = inputs.texts.as_deref()
        && let Some(first) = texts.first()
    {
        return Ok(wire::object(vec![(
            "choices",
            wire::array(vec![wire::object(vec![
                ("index", wire::integer(0)),
                (
                    "message",
                    wire::object(vec![
                        ("role", wire::text("assistant")),
                        ("content", wire::text(first)),
                    ]),
                ),
                ("finish_reason", wire::text("stop")),
            ])]),
        )]));
    }
    if let Some(tool_calls) = inputs.tool_calls.as_ref()
        && !tool_calls.is_empty()
    {
        return wire::to_json(tool_calls);
    }
    Ok(wire::object(Vec::new()))
}

// LiteLLM replaces the messages with the endpoint's evaluation and rebuilds the scanned texts
fn write_request(
    inputs: &mut GuardrailApiInputs,
    output: &GuardrailValue,
) -> Result<(), EvaluationError> {
    let GuardrailValue::Object(fields) = output else {
        return Err(EvaluationError::Shape { field: "messages" });
    };
    let messages = match fields.get("messages") {
        Some(GuardrailValue::Array(messages)) => messages.clone(),
        _ => Vec::new(),
    };
    let mut structured = Vec::with_capacity(messages.len());
    let mut texts = Vec::new();
    for message in &messages {
        let converted = as_guardrail_message(message)?;
        if let Some(content) = converted.extra.get("content") {
            match content {
                GuardrailValue::Array(parts) => {
                    let parts_texts: Vec<String> = parts.iter().filter_map(text_part).collect();
                    if !parts_texts.is_empty() {
                        texts.push(parts_texts.join(" "));
                    }
                }
                GuardrailValue::Text(text) if !text.is_empty() => texts.push(text.clone()),
                _ => {}
            }
        }
        structured.push(converted);
    }
    inputs.structured_messages = Some(structured);
    inputs.texts = Some(texts);
    Ok(())
}

fn write_response(
    inputs: &mut GuardrailApiInputs,
    output: &GuardrailValue,
) -> Result<(), EvaluationError> {
    if inputs.texts.as_ref().is_some_and(|texts| !texts.is_empty()) {
        inputs.texts = Some(vec![last_choice_content(output)]);
    } else if inputs
        .tool_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty())
    {
        let GuardrailValue::Array(items) = output else {
            return Err(EvaluationError::Shape {
                field: "tool_calls",
            });
        };
        inputs.tool_calls = Some(items.clone());
    }
    Ok(())
}

fn last_choice_content(output: &GuardrailValue) -> String {
    let GuardrailValue::Object(body) = output else {
        return String::new();
    };
    let Some(GuardrailValue::Array(choices)) = body.get("choices") else {
        return String::new();
    };
    let Some(GuardrailValue::Object(choice)) = choices.last() else {
        return String::new();
    };
    let Some(GuardrailValue::Object(message)) = choice.get("message") else {
        return String::new();
    };
    match message.get("content") {
        Some(GuardrailValue::Text(content)) => content.clone(),
        _ => String::new(),
    }
}
