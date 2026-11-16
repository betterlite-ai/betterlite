use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::session::session_id_from_request;
use crate::straiker::delivery::request_json::{DeliveryError, json_value};
use crate::straiker::delivery::settings::StraikerSettings;
use crate::straiker::delivery::{ApplyRequest, DeliveryCall, LITELLM_VERSION, phase_name};
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

#[derive(Serialize)]
pub struct Envelope {
    pub schema_version: &'static str,
    pub event: Event,
    pub request: StraikerWebhookContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<StraikerWebhookContent>,
    pub context: StraikerWebhookContext,
    pub identity: StraikerWebhookIdentity,
    pub application: WebhookApplication,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
}

#[derive(Serialize)]
pub struct WebhookApplication {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

struct Shared {
    event_id: String,
    content: StraikerWebhookContent,
    context: StraikerWebhookContext,
    identity: StraikerWebhookIdentity,
    application: WebhookApplication,
    metadata: Option<Object>,
}

struct ResponseFields {
    finish_reason: Option<String>,
    usage: Option<StraikerWebhookUsage>,
    request_messages: Option<Vec<Object>>,
    streamed: bool,
}

pub fn build(
    settings: &StraikerSettings,
    request: &ApplyRequest<'_>,
    object: &Object,
) -> Result<Envelope, DeliveryError> {
    let shared = shared(settings, request, object);
    match request.input_type {
        GuardrailInputType::Request => Ok(request_envelope(shared)),
        GuardrailInputType::Response => {
            let fields = response_fields(request, object)?;
            Ok(response_envelope(shared, fields))
        }
    }
}

fn shared(settings: &StraikerSettings, request: &ApplyRequest<'_>, object: &Object) -> Shared {
    let call = request.call;
    let model = model_of(&request.inputs, object);
    let event_id = format!(
        "{}:{}",
        call.litellm_call_id.unwrap_or("litellm"),
        phase_name(request.input_type)
    );
    Shared {
        event_id,
        content: content_of(&request.inputs),
        context: context(
            settings,
            call,
            object,
            model.as_deref(),
            session_id_from_request(request.request_data),
        ),
        identity: extract_identity(object),
        application: application(settings, object),
        metadata: webhook_metadata(object, &settings.metadata),
    }
}

fn request_envelope(shared: Shared) -> Envelope {
    let Shared {
        event_id,
        content,
        context,
        identity,
        application,
        metadata,
    } = shared;
    Envelope {
        schema_version: SCHEMA_VERSION,
        event: Event {
            event_type: "pre_call",
            id: event_id,
            stream: Stream {
                phase: "none",
                index: None,
            },
        },
        request: content,
        response: None,
        context,
        identity,
        application,
        usage: None,
        metadata,
    }
}

fn response_envelope(shared: Shared, fields: ResponseFields) -> Envelope {
    let Shared {
        event_id,
        content,
        context,
        identity,
        application,
        metadata,
    } = shared;
    let ResponseFields {
        finish_reason,
        usage,
        request_messages,
        streamed,
    } = fields;
    let phase = if streamed { "assembled" } else { "none" };
    Envelope {
        schema_version: SCHEMA_VERSION,
        event: Event {
            event_type: "post_call",
            id: event_id,
            stream: Stream { phase, index: None },
        },
        request: StraikerWebhookContent {
            texts: Vec::new(),
            images: Vec::new(),
            structured_messages: request_messages,
            tools: None,
            tool_calls: None,
            finish_reason: None,
        },
        response: Some(StraikerWebhookContent {
            finish_reason,
            ..content
        }),
        context,
        identity,
        application,
        usage,
        metadata,
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

fn response_fields(
    request: &ApplyRequest<'_>,
    object: &Object,
) -> Result<ResponseFields, DeliveryError> {
    let response = request
        .request_data
        .get("response")
        .map(json_value)
        .transpose()?;
    Ok(ResponseFields {
        finish_reason: response.as_ref().and_then(response_finish_reason),
        usage: response.as_ref().and_then(response_usage),
        request_messages: request_structured_messages(
            object,
            request.call.routes,
            request.call.mappings,
        )
        .filter(|rows| !rows.is_empty()),
        streamed: request_is_streamed(object),
    })
}

pub fn model_of(inputs: &GuardrailApiInputs, request_data: &Object) -> Option<String> {
    inputs.model.clone().or_else(|| {
        request_data
            .get("model")
            .and_then(Json::as_str)
            .map(str::to_owned)
    })
}

#[must_use]
pub fn context(
    settings: &StraikerSettings,
    call: &DeliveryCall<'_>,
    request_data: &Object,
    model: Option<&str>,
    session_id: Option<String>,
) -> StraikerWebhookContext {
    StraikerWebhookContext {
        call_surface: call_surface(call.call_type, request_data),
        mode: settings.configured_modes.clone(),
        model: model.map(str::to_owned),
        model_provider: model_provider(request_data, model, call.provider_names),
        destination: destination_host(request_data),
        session_id,
        litellm_call_id: call.litellm_call_id.map(str::to_owned),
        litellm_trace_id: call.litellm_trace_id.map(str::to_owned),
        litellm_version: Some(LITELLM_VERSION.to_owned()),
    }
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

fn application(settings: &StraikerSettings, request_data: &Object) -> WebhookApplication {
    let metadata = merged_metadata(request_data);
    let text = |key: &str| {
        metadata
            .get(key)
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    WebhookApplication {
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
