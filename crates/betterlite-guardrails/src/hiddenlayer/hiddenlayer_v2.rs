use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use reqwest::Client;
use serde::Serialize;
use uuid::Uuid;

use crate::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::hiddenlayer::evaluation::error::{EvaluationError, blocked};
use crate::hiddenlayer::evaluation::{
    BLOCK_MESSAGE, EvaluationArgs, VIOLATED_POLICY, as_guardrail_message, detection_headers,
    evaluation_timeout, guardrail_settings, metadata, text_part, wire,
};
use crate::hiddenlayer::hiddenlayer_authentication::HiddenlayerAuth;
use crate::hiddenlayer::hiddenlayer_authentication::detection::{
    DetectionRequest, DetectionResponse,
};
use crate::hiddenlayer::hiddenlayer_authentication::token::JwtTransport;
use crate::primitives::json_value::JsonValue;

mod tests;

pub const VERSION: &str = "2";

// every request part is sent, null when the input lacks it
#[derive(Serialize)]
struct RequestPayload<'request> {
    messages: &'request Option<Vec<GuardrailMessage>>,
    model: &'request Option<String>,
    tools: &'request Option<Vec<GuardrailToolParam>>,
}

#[derive(Serialize)]
struct ChoicesPayload<'choices> {
    choices: [Choice<'choices>; 1],
}

#[derive(Serialize)]
struct Choice<'choice> {
    index: u8,
    message: ChoiceMessage<'choice>,
    finish_reason: &'static str,
}

#[derive(Serialize)]
struct ChoiceMessage<'message> {
    role: &'static str,
    content: &'message str,
}

// LiteLLM's HiddenlayerGuardrailV2: the whole request or response payload is evaluated
pub struct HiddenlayerV2Guardrail {
    pub base: GuardrailBase,
    pub auth: HiddenlayerAuth,
    pub timeout: Duration,
    pub new_id: fn() -> Uuid,
}

impl HiddenlayerV2Guardrail {
    pub fn new<L>(
        args: &EvaluationArgs,
        environment: &L,
        client: Client,
        transport: Arc<dyn JwtTransport>,
        new_id: fn() -> Uuid,
    ) -> Result<Self, EvaluationError>
    where
        L: EnvironmentLookup,
    {
        let auth = HiddenlayerAuth::new(&args.credentials(), environment, client, transport)?;
        let timeout = evaluation_timeout(args.timeout)?;
        let base = GuardrailBase::new(guardrail_settings(args, None), true)?;
        Ok(Self {
            base,
            auth,
            timeout,
            new_id,
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
        if action.eq_ignore_ascii_case("block") {
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
            GuardrailInputType::Request => "detection/v2/request-evaluations",
            GuardrailInputType::Response => "detection/v2/response-evaluations",
        };
        let request = DetectionRequest {
            path,
            headers: detection_headers(VERSION, None, Some(hl_headers))?,
            body: payload,
        };
        self.auth
            .post_detection_with_timeout(&request, self.timeout)
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
            && let Some(GuardrailValue::Object(proxy_headers)) = proxy.get_mut("headers")
        {
            proxy_headers.insert(
                metadata::ROUNDTRIP_HEADER.to_owned(),
                GuardrailValue::Text(id.clone()),
            );
            hl_headers.insert(metadata::ROUNDTRIP_HEADER.to_owned(), id);
        }
    }
    hl_headers
}

pub fn request_payload(inputs: &GuardrailApiInputs) -> Result<JsonValue, EvaluationError> {
    wire::to_json(&RequestPayload {
        messages: &inputs.structured_messages,
        model: &inputs.model,
        tools: &inputs.tools,
    })
}

pub fn response_payload(inputs: &GuardrailApiInputs) -> Result<JsonValue, EvaluationError> {
    if let Some(first) = inputs.texts.as_deref().and_then(|texts| texts.first()) {
        return wire::to_json(&ChoicesPayload {
            choices: [Choice {
                index: 0,
                message: ChoiceMessage {
                    role: "assistant",
                    content: first,
                },
                finish_reason: "stop",
            }],
        });
    }
    if let Some(tool_calls) = inputs.tool_calls.as_ref()
        && !tool_calls.is_empty()
    {
        return wire::to_json(tool_calls);
    }
    Ok(JsonValue::Object(IndexMap::new()))
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
        if let Some(text) = converted.extra.get("content").and_then(message_text) {
            texts.push(text);
        }
        structured.push(converted);
    }
    inputs.structured_messages = Some(structured);
    inputs.texts = Some(texts);
    Ok(())
}

// A list content joins its text parts with a space; an empty text or no text part adds nothing
fn message_text(content: &GuardrailValue) -> Option<String> {
    match content {
        GuardrailValue::Array(parts) => {
            let texts: Vec<String> = parts.iter().filter_map(text_part).collect();
            (!texts.is_empty()).then(|| texts.join(" "))
        }
        GuardrailValue::Text(text) => (!text.is_empty()).then(|| text.clone()),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_) => None,
    }
}

fn write_response(
    inputs: &mut GuardrailApiInputs,
    output: &GuardrailValue,
) -> Result<(), EvaluationError> {
    let has_texts = inputs.texts.as_ref().is_some_and(|texts| !texts.is_empty());
    let has_calls = inputs
        .tool_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty());
    match (has_texts, has_calls) {
        (true, _) => inputs.texts = Some(vec![last_choice_content(output)?]),
        (false, true) => {
            let GuardrailValue::Array(items) = output else {
                return Err(EvaluationError::Shape {
                    field: "tool_calls",
                });
            };
            inputs.tool_calls = Some(items.clone());
        }
        (false, false) => {}
    }
    Ok(())
}

// LiteLLM reads choices[-1]; a missing or null choices, message or content reads as an empty content, an empty list fails
fn last_choice_content(output: &GuardrailValue) -> Result<String, EvaluationError> {
    let GuardrailValue::Object(body) = output else {
        return Err(EvaluationError::Shape { field: "choices" });
    };
    let list = match body.get("choices") {
        None | Some(GuardrailValue::Null) => return Ok(String::new()),
        Some(GuardrailValue::Array(list)) => list,
        Some(_) => return Err(EvaluationError::Shape { field: "choices" }),
    };
    let Some(GuardrailValue::Object(choice)) = list.last() else {
        return Err(EvaluationError::Shape { field: "choices" });
    };
    let message = match choice.get("message") {
        Some(GuardrailValue::Null) | None => return Ok(String::new()),
        Some(GuardrailValue::Object(message)) => message,
        Some(_) => {
            return Err(EvaluationError::Shape {
                field: "choices.message",
            });
        }
    };
    match message.get("content") {
        Some(GuardrailValue::Text(content)) => Ok(content.clone()),
        None | Some(GuardrailValue::Null) => Ok(String::new()),
        Some(_) => Err(EvaluationError::Shape {
            field: "choices.message.content",
        }),
    }
}
