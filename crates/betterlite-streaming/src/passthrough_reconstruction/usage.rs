use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::Json;
use serde::Serialize;
use uuid::Uuid;

use crate::passthrough_reconstruction::converter::UsageCalculator;
use crate::passthrough_reconstruction::error::{EventError, ReconstructionError};
use crate::passthrough_reconstruction::events::{
    RawSseChunk, extract_sse_data, integer_field, object_field, split_sse_chunk_into_events,
    text_field,
};
use crate::stream_chunk_shaping::records::JsonObject;
use crate::stream_usage_assembly::wire::Usage;
use crate::streamed_response_assembly::response::{
    ChatChoice, ChatMessage, ChatResponse, Response,
};

pub struct UsageOnlyRequest<'request, Calculator>
where
    Calculator: UsageCalculator,
{
    pub chunks: &'request [RawSseChunk],
    pub model: &'request str,
    pub speed: Option<&'request str>,
    pub calculator: &'request Calculator,
    pub now: i64,
    pub next_id: fn() -> Uuid,
}

// The wire object mirrors the dict LiteLLM hands calculate_usage, key for key.
#[derive(Serialize)]
struct UsageWire<'usage> {
    input_tokens: i64,
    output_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation: Option<CacheCreationWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server_tool_use: Option<ServerToolUseWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_geo: Option<&'usage str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<&'usage str>,
}

#[derive(Serialize)]
struct CacheCreationWire {
    ephemeral_5m_input_tokens: i64,
    ephemeral_1h_input_tokens: i64,
}

#[derive(Serialize)]
struct ServerToolUseWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    web_search_requests: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_search_requests: Option<i64>,
}

struct UsageTotals {
    input_tokens: i64,
    output_tokens: i64,
    cache_read: i64,
    cache_creation: i64,
    cache_creation_5m: Option<i64>,
    cache_creation_1h: Option<i64>,
    web_search_requests: Option<i64>,
    tool_search_requests: Option<i64>,
    inference_geo: Option<String>,
    speed: Option<String>,
    stop_reason: Option<String>,
    model: String,
    found: bool,
}

pub fn build_usage_only_response<Calculator>(
    request: &UsageOnlyRequest<'_, Calculator>,
) -> Result<Option<Response>, ReconstructionError<Calculator::Failure>>
where
    Calculator: UsageCalculator,
{
    let totals = collect_usage(request.chunks, request.model)?;
    if !totals.found {
        return Ok(None);
    }
    let usage_object = totals.usage_object()?;
    let usage = request
        .calculator
        .calculate_usage(&usage_object, request.speed)
        .map_err(|source| ReconstructionError::Usage { source })?;
    Ok(Some(usage_only_response(request, totals, usage)))
}

fn collect_usage(chunks: &[RawSseChunk], model: &str) -> Result<UsageTotals, EventError> {
    let mut totals = UsageTotals {
        input_tokens: 0,
        output_tokens: 0,
        cache_read: 0,
        cache_creation: 0,
        cache_creation_5m: None,
        cache_creation_1h: None,
        web_search_requests: None,
        tool_search_requests: None,
        inference_geo: None,
        speed: None,
        stop_reason: None,
        model: model.to_owned(),
        found: false,
    };
    for chunk in chunks {
        for event in split_sse_chunk_into_events(chunk)? {
            let Some(data) = extract_sse_data(&event) else {
                continue;
            };
            match text_field(&data, "type") {
                Some("message_start") => totals.message_start(&data),
                Some("message_delta") => totals.message_delta(&data),
                _ => {}
            }
        }
    }
    Ok(totals)
}

fn usage_only_response<Calculator>(
    request: &UsageOnlyRequest<'_, Calculator>,
    totals: UsageTotals,
    usage: Usage,
) -> Response
where
    Calculator: UsageCalculator,
{
    let finish_reason = totals.stop_reason.as_deref().map_or_else(
        || "stop".to_owned(),
        |reason| map_finish_reason(reason).reason.as_str().to_owned(),
    );
    let id = (request.next_id)();
    let response = ChatResponse {
        id: format!("chatcmpl-{id}"),
        object: "chat.completion".to_owned(),
        created: request.now,
        model: totals.model,
        system_fingerprint: None,
        service_tier: None,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessage {
                content: Some(String::new()),
                role: "assistant".to_owned(),
                tool_calls: None,
                function_call: None,
                provider_specific_fields: None,
                reasoning_content: None,
                thinking_blocks: None,
                annotations: None,
                audio: None,
                images: None,
            },
            finish_reason,
            provider_specific_fields: None,
        }],
        usage,
        hidden_params: JsonObject::new(),
    };
    Response::Chat(Box::new(response))
}

fn overwrite(target: &mut i64, usage: &Json, key: &str) {
    if let Some(value) = integer_field(usage, key) {
        *target = value;
    }
}

fn overwrite_some(target: &mut Option<i64>, usage: &Json, key: &str) {
    if let Some(value) = integer_field(usage, key) {
        *target = Some(value);
    }
}

impl UsageTotals {
    fn message_start(&mut self, data: &Json) {
        let message = object_field(data, "message");
        if self.model.is_empty() || self.model == "unknown" {
            let streamed = message
                .and_then(|fields| text_field(fields, "model"))
                .filter(|model| !model.is_empty());
            if let Some(model) = streamed {
                model.clone_into(&mut self.model);
            }
        }
        if let Some(usage) = message.and_then(|fields| object_field(fields, "usage")) {
            overwrite(&mut self.input_tokens, usage, "input_tokens");
            overwrite(&mut self.cache_read, usage, "cache_read_input_tokens");
            overwrite(
                &mut self.cache_creation,
                usage,
                "cache_creation_input_tokens",
            );
            if let Some(splits) = object_field(usage, "cache_creation") {
                self.cache_creation_5m = integer_field(splits, "ephemeral_5m_input_tokens");
                self.cache_creation_1h = integer_field(splits, "ephemeral_1h_input_tokens");
            }
            self.take_labels(usage);
            overwrite(&mut self.output_tokens, usage, "output_tokens");
        }
        self.found = true;
    }

    fn message_delta(&mut self, data: &Json) {
        let delta = object_field(data, "delta");
        let stop = delta
            .and_then(|fields| text_field(fields, "stop_reason"))
            .filter(|reason| !reason.is_empty());
        if let Some(reason) = stop {
            self.stop_reason = Some(reason.to_owned());
        }
        if let Some(usage) = object_field(data, "usage") {
            overwrite(&mut self.output_tokens, usage, "output_tokens");
            if let Some(server) = object_field(usage, "server_tool_use") {
                overwrite_some(&mut self.web_search_requests, server, "web_search_requests");
                overwrite_some(
                    &mut self.tool_search_requests,
                    server,
                    "tool_search_requests",
                );
            }
            overwrite(&mut self.cache_read, usage, "cache_read_input_tokens");
            self.take_labels(usage);
        }
        self.found = true;
    }

    fn take_labels(&mut self, usage: &Json) {
        if let Some(geo) = text_field(usage, "inference_geo") {
            self.inference_geo = Some(geo.to_owned());
        }
        if let Some(speed) = text_field(usage, "speed") {
            self.speed = Some(speed.to_owned());
        }
    }

    fn usage_object(&self) -> Result<Json, serde_json::Error> {
        let derived = self
            .cache_creation_5m
            .unwrap_or(0)
            .saturating_add(self.cache_creation_1h.unwrap_or(0));
        let cache_creation = if self.cache_creation == 0 && derived != 0 {
            derived
        } else {
            self.cache_creation
        };
        let wire = UsageWire {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_input_tokens: (self.cache_read != 0).then_some(self.cache_read),
            cache_creation_input_tokens: (cache_creation != 0).then_some(cache_creation),
            cache_creation: (self.cache_creation_5m.is_some() || self.cache_creation_1h.is_some())
                .then_some(CacheCreationWire {
                    ephemeral_5m_input_tokens: self.cache_creation_5m.unwrap_or(0),
                    ephemeral_1h_input_tokens: self.cache_creation_1h.unwrap_or(0),
                }),
            server_tool_use: (self.web_search_requests.is_some()
                || self.tool_search_requests.is_some())
            .then_some(ServerToolUseWire {
                web_search_requests: self.web_search_requests,
                tool_search_requests: self.tool_search_requests,
            }),
            inference_geo: self.inference_geo.as_deref(),
            speed: self.speed.as_deref(),
        };
        let text = serde_json::to_string(&wire)?;
        serde_json::from_str(&text)
    }
}
