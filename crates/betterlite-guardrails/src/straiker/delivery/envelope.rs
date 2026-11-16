use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::structured::MappedHandler;
use serde::Serialize;
use serde_json::Number;
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::session::session_id_from_request;
use crate::straiker::delivery::settings::StraikerSettings;
use crate::straiker::delivery::{ApplyRequest, LITELLM_VERSION, phase_name};
use crate::straiker::v3::answer_shape::Response;
use crate::straiker::v3::completion_as_chat::TextCompletionResponse;
use crate::straiker_webhook::context::{call_surface, destination_host, model_provider};
use crate::straiker_webhook::jsonable::record_dict;
use crate::straiker_webhook::metadata::{extract_identity, merged_metadata, webhook_metadata};
use crate::straiker_webhook::responses::{response_finish_reason, response_usage};
use crate::straiker_webhook::streamed::request_is_streamed;
use crate::straiker_webhook::structured_messages::request_structured_messages;
use crate::straiker_webhook::types::{
    StraikerWebhookContent, StraikerWebhookContext, StraikerWebhookIdentity, StraikerWebhookUsage,
};

pub const SCHEMA_VERSION: &str = "1";
const TEXT_COMPLETION_OBJECT: &str = "text_completion";

#[derive(Debug, Error)]
pub enum DeliveryError {
    #[error("the value {value} is not a JSON number")]
    NotJsonNumber { value: f64 },
    #[error("the request data could not be rendered as JSON")]
    Serialize(#[source] serde_json::Error),
}

pub fn json_object(data: &BTreeMap<String, GuardrailValue>) -> Result<Object, DeliveryError> {
    data.iter()
        .map(|(key, value)| Ok((key.clone(), json_value(value)?)))
        .collect()
}

pub fn json_value(value: &GuardrailValue) -> Result<Json, DeliveryError> {
    match value {
        GuardrailValue::Null => Ok(Json::Null),
        GuardrailValue::Boolean(flag) => Ok(Json::Bool(*flag)),
        GuardrailValue::SignedInteger(number) => Ok(Json::Number(Number::from(*number))),
        GuardrailValue::UnsignedInteger(number) => Ok(Json::Number(Number::from(*number))),
        GuardrailValue::Decimal(number) => Number::from_f64(*number)
            .map(Json::Number)
            .ok_or(DeliveryError::NotJsonNumber { value: *number }),
        GuardrailValue::Text(text) => Ok(Json::Text(text.clone())),
        GuardrailValue::Array(items) => items
            .iter()
            .map(json_value)
            .collect::<Result<Vec<Json>, DeliveryError>>()
            .map(Json::List),
        GuardrailValue::Object(fields) => json_object(fields).map(Json::Object),
    }
}

// LiteLLM's StraikerWebhookRequest model: the nine keys a webhook body carries, unset ones left out.
#[derive(Serialize)]
pub struct Envelope {
    pub schema_version: &'static str,
    pub event: Event,
    pub request: StraikerWebhookContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<StraikerWebhookContent>,
    pub context: StraikerWebhookContext,
    pub identity: StraikerWebhookIdentity,
    pub application: Application,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<StraikerWebhookUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Object>,
}

#[derive(Serialize)]
pub struct Event {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub id: String,
    pub stream: Stream,
}

#[derive(Serialize)]
pub struct Stream {
    pub phase: &'static str,
}

#[derive(Serialize)]
pub struct Application {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn event(request: &ApplyRequest<'_>, object: &Object) -> Event {
    let (event_type, phase) = match request.input_type {
        GuardrailInputType::Request => ("pre_call", "none"),
        GuardrailInputType::Response if request_is_streamed(object) => ("post_call", "assembled"),
        GuardrailInputType::Response => ("post_call", "none"),
    };
    Event {
        event_type,
        id: format!(
            "{}:{}",
            request.call.litellm_call_id.unwrap_or("litellm"),
            phase_name(request.input_type)
        ),
        stream: Stream { phase },
    }
}

pub fn build(
    settings: &StraikerSettings,
    request: &ApplyRequest<'_>,
    object: &Object,
) -> Result<Envelope, DeliveryError> {
    let mut envelope = Envelope {
        schema_version: SCHEMA_VERSION,
        event: event(request, object),
        request: content_of(&request.inputs),
        response: None,
        context: context(settings, request, object),
        identity: extract_identity(object),
        application: application(settings, object),
        usage: None,
        metadata: webhook_metadata(object, &settings.metadata),
    };
    if request.input_type == GuardrailInputType::Request {
        return Ok(envelope);
    }
    let response = request
        .request_data
        .get("response")
        .map(json_value)
        .transpose()?;
    envelope.request = StraikerWebhookContent {
        texts: Vec::new(),
        images: Vec::new(),
        structured_messages: request_messages(request, object).filter(|rows| !rows.is_empty()),
        tools: None,
        tool_calls: None,
        finish_reason: None,
    };
    let mut answer = content_of(&request.inputs);
    answer.finish_reason = response.as_ref().and_then(response_finish_reason);
    envelope.response = Some(answer);
    envelope.usage = response.as_ref().and_then(response_usage);
    Ok(envelope)
}

// The model the hook sees: the guardrail input's, else the request's.
#[must_use]
pub fn context(
    settings: &StraikerSettings,
    request: &ApplyRequest<'_>,
    request_data: &Object,
) -> StraikerWebhookContext {
    let call = request.call;
    let model = request.inputs.model.clone().or_else(|| {
        request_data
            .get("model")
            .and_then(Json::as_str)
            .map(str::to_owned)
    });
    StraikerWebhookContext {
        call_surface: call_surface(call.call_type, request_data),
        mode: settings.configured_modes.clone(),
        model: model.clone(),
        model_provider: model_provider(request_data, model.as_deref(), call.provider_names),
        destination: destination_host(request_data),
        session_id: session_id_from_request(request.request_data),
        litellm_call_id: call.litellm_call_id.map(str::to_owned),
        litellm_trace_id: call.litellm_trace_id.map(str::to_owned),
        litellm_version: Some(LITELLM_VERSION.to_owned()),
    }
}

fn content_of(inputs: &GuardrailApiInputs) -> StraikerWebhookContent {
    StraikerWebhookContent {
        texts: inputs.texts.clone().unwrap_or_default(),
        images: inputs.images.clone().unwrap_or_default(),
        structured_messages: opaque_list(inputs.structured_messages.as_deref()),
        tools: opaque_list(inputs.tools.as_deref()),
        tool_calls: opaque_list(inputs.tool_calls.as_deref()),
        finish_reason: None,
    }
}

fn request_messages(request: &ApplyRequest<'_>, object: &Object) -> Option<Vec<Object>> {
    let handlers: Vec<MappedHandler<'_, CallType>> = request
        .call
        .mappings
        .iter()
        .map(|&(call_types, handler)| MappedHandler {
            call_types,
            handler,
        })
        .collect();
    request_structured_messages(object, request.call.routes, &handlers)
}

// The proxy's answer as the shape the client spoke; a v3 detect turns a Messages answer back from the chat completion the proxy rebuilt.
pub fn response_shape(request_data: &Object) -> Result<Response, DeliveryError> {
    let Some(response) = request_data.get("response") else {
        return Ok(Response::Other(Json::Null));
    };
    let Json::Object(fields) = response else {
        return Ok(Response::Other(response.clone()));
    };
    if fields.get("object").and_then(Json::as_str) == Some(TEXT_COMPLETION_OBJECT) {
        let encoded =
            serde_json::to_value(Json::Object(fields.clone())).map_err(DeliveryError::Serialize)?;
        if let Ok(completion) = serde_json::from_value::<TextCompletionResponse>(encoded) {
            return Ok(Response::TextCompletion(completion));
        }
    }
    Ok(Response::Chat(fields.clone()))
}

fn application(settings: &StraikerSettings, request_data: &Object) -> Application {
    let metadata = merged_metadata(request_data);
    let text = |key: &str| {
        metadata
            .get(key)
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    Application {
        source: text("agent_id").unwrap_or_else(|| settings.source.clone()),
        name: text("app_name"),
    }
}

fn opaque_list<Record>(items: Option<&[Record]>) -> Option<Vec<Object>>
where
    Record: Serialize,
{
    let rows: Vec<Object> = items?.iter().filter_map(record_dict).collect();
    if rows.is_empty() {
        return None;
    }
    Some(rows)
}
