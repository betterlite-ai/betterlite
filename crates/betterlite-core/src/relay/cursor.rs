use std::sync::Arc;

use betterlite_http::user_url::error::InvalidUrl;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use percent_encoding::percent_decode_str;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::api::media_relay_logging::spend::decimal_json;

mod tests;

const PROVIDER: &str = "cursor";

// LiteLLM's CURSOR_AGENT_ENDPOINTS keeps this order and stops at the first pattern that matches.
const AGENT_ENDPOINTS: [(&str, &str, &str); 10] = [
    ("POST", "/v0/agents", "cursor:agent:create"),
    ("GET", "/v0/agents", "cursor:agent:list"),
    ("POST", "/v0/agents/{id}/followup", "cursor:agent:followup"),
    ("POST", "/v0/agents/{id}/stop", "cursor:agent:stop"),
    ("DELETE", "/v0/agents/{id}", "cursor:agent:delete"),
    (
        "GET",
        "/v0/agents/{id}/conversation",
        "cursor:agent:conversation",
    ),
    ("GET", "/v0/agents/{id}", "cursor:agent:status"),
    ("GET", "/v0/me", "cursor:account:info"),
    ("GET", "/v0/models", "cursor:models:list"),
    ("GET", "/v0/repositories", "cursor:repositories:list"),
];

pub struct CursorPassthroughInput<'call> {
    pub logging_obj: &'call StreamLoggingObject,
    pub method: &'call str,
    pub url_route: &'call str,
    pub response_body: &'call Json,
    pub request_body: &'call Json,
    pub result: &'call str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub kwargs: PassThroughLoggingKwargs,
}

#[derive(Debug, Error)]
enum CursorPassthroughError {
    #[error("the Cursor request URL could not be read: {source}")]
    Route {
        #[source]
        source: InvalidUrl,
    },
}

pub struct CursorPassthroughLogger {
    payload_builder: Arc<dyn StandardPayloadBuilder>,
}

impl CursorPassthroughLogger {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - B3606 constructs the logger through this fixed name
    pub fn new(payload_builder: Arc<dyn StandardPayloadBuilder>) -> Self {
        Self { payload_builder }
    }

    // LiteLLM wraps the body in try/except: the failure logs its error and returns the relayed result with the caller's kwargs.
    #[must_use]
    pub fn cursor_passthrough_handler(&self, input: CursorPassthroughInput<'_>) -> HandlerOutcome {
        match self.run(&input) {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::error!("Error in Cursor passthrough logging handler: {error}");
                HandlerOutcome {
                    result: Some(standard_result(input.result)),
                    kwargs: input.kwargs,
                }
            }
        }
    }

    fn run(
        &self,
        input: &CursorPassthroughInput<'_>,
    ) -> Result<HandlerOutcome, CursorPassthroughError> {
        let operation = classify_request(input.method, &request_path(input.url_route)?);
        let model_name = format!("cursor/{operation}");
        let response_summary = agent_summary(input.response_body, input.result);
        let mut kwargs = record(input, &model_name);
        let init_response = response_object(&response_summary);
        let payload = self.build_payload(input, &init_response);
        kwargs.entries.insert(
            STANDARD_LOGGING_OBJECT.to_owned(),
            payload_entry(payload.as_ref()),
        );
        tracing::debug!(
            operation = %operation,
            agent_id = %agent_field(input.response_body, "id"),
            "Cursor passthrough logging",
        );
        Ok(HandlerOutcome {
            result: Some(standard_result(&response_summary)),
            kwargs,
        })
    }

    fn build_payload(
        &self,
        input: &CursorPassthroughInput<'_>,
        init_response: &Json,
    ) -> Option<StandardLoggingPayload> {
        input
            .logging_obj
            .read(&mut |state| {
                self.payload_builder
                    .build(state, init_response, input.start_time, input.end_time)
            })
            .flatten()
    }
}

// LiteLLM writes the provider into model_call_details alone, so the payload leaves custom_llm_provider empty.
fn record(input: &CursorPassthroughInput<'_>, model_name: &str) -> PassThroughLoggingKwargs {
    let model = Json::Text(model_name.to_owned());
    let zero_cost = decimal_json(Decimal::ZERO);
    let mut kwargs = input.kwargs.clone();
    kwargs.entries.insert("model".to_owned(), model.clone());
    kwargs
        .entries
        .insert("response_cost".to_owned(), zero_cost.clone());
    let details = IndexMap::from([
        ("model".to_owned(), model),
        (
            "custom_llm_provider".to_owned(),
            Json::Text(PROVIDER.to_owned()),
        ),
        ("response_cost".to_owned(), zero_cost),
    ]);
    input.logging_obj.edit(&mut |state| {
        model_name.clone_into(&mut state.model);
        for (key, value) in &details {
            state.model_call_details.insert(key.clone(), value.clone());
        }
    });
    kwargs
}

fn standard_result(summary: &str) -> PassThroughResponse {
    PassThroughResponse::Standard(PassThroughResponseObject {
        response: Json::Text(summary.to_owned()),
    })
}

fn response_object(summary: &str) -> Json {
    Json::Object(IndexMap::from([(
        "response".to_owned(),
        Json::Text(summary.to_owned()),
    )]))
}

// A payload that cannot be written as JSON stands in as Null, the way a failed build does in LiteLLM.
fn payload_entry(payload: Option<&StandardLoggingPayload>) -> Json {
    payload
        .and_then(|built| serde_json::to_string(built).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Json::Null)
}

// httpx.URL().path is the percent-decoded path with an empty one read as "/".
fn request_path(url_route: &str) -> Result<String, CursorPassthroughError> {
    let url =
        HttpxUrl::parse(url_route).map_err(|source| CursorPassthroughError::Route { source })?;
    let path = if url.path.is_empty() {
        "/"
    } else {
        url.path.as_str()
    };
    Ok(percent_decode_str(path).decode_utf8_lossy().into_owned())
}

fn classify_request(method: &str, path: &str) -> String {
    let normalized = path.trim_end_matches('/');
    let upper = method.to_uppercase();
    for (endpoint_method, endpoint_path, operation) in AGENT_ENDPOINTS {
        if upper == endpoint_method && path_matches(endpoint_path, normalized) {
            return operation.to_owned();
        }
    }
    format!("cursor:{}:{normalized}", method.to_lowercase())
}

fn path_matches(pattern: &str, request: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.trim_matches('/').split('/').collect();
    let request_parts: Vec<&str> = request.trim_matches('/').split('/').collect();
    pattern_parts.len() == request_parts.len()
        && pattern_parts
            .iter()
            .zip(&request_parts)
            .all(|(pattern_part, request_part)| {
                let wildcard = pattern_part.starts_with('{') && pattern_part.ends_with('}');
                wildcard || pattern_part == request_part
            })
}

// Python's truthiness drops an absent, empty or non-string field, so only a JSON string names a part.
fn agent_field<'body>(body: &'body Json, name: &'static str) -> &'body str {
    let Json::Object(fields) = body else {
        return "";
    };
    let Some(Json::Text(text)) = fields.get(name) else {
        return "";
    };
    text.as_str()
}

fn agent_summary(body: &Json, result: &str) -> String {
    let mut parts = Vec::new();
    for (label, value) in [
        ("id", agent_field(body, "id")),
        ("name", agent_field(body, "name")),
        ("status", agent_field(body, "status")),
    ] {
        if !value.is_empty() {
            parts.push(format!("{label}={value}"));
        }
    }
    if parts.is_empty() {
        return result.to_owned();
    }
    parts.join(", ")
}
