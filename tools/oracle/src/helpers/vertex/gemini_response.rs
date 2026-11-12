use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_vertex::gemini_response::complete::{
    CompleteRequest, CompleteResponse, RawResponse, transform_response,
};
use betterlite_vertex::gemini_response_parts::projection::CallMode;
use betterlite_vertex::model_policy::error::VertexAiError;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    legacy_function: bool,
    now: i64,
    generated_id: String,
    headers: BTreeMap<String, String>,
    body: Json,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let uuid = Uuid::parse_str(&input.generated_id).or_value_error()?;
        let body = serde_json::to_vec(&input.body).or_value_error()?;
        let headers = header_map(&input.headers)?;
        let raw = RawResponse {
            headers: &headers,
            body: &body,
        };
        let request = CompleteRequest {
            model: &input.model,
            now: input.now,
            call_mode: if input.legacy_function {
                CallMode::LegacyFunction
            } else {
                CallMode::Tools
            },
        };
        let mut ids = || uuid;
        let response = transform_response(&raw, &request, &mut ids).map_err(vertex_error)?;
        Ok(state(response))
    }
}

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).or_value_error()?,
            HeaderValue::from_str(value).or_value_error()?,
        );
    }
    Ok(map)
}

fn vertex_error(error: VertexAiError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "VertexAIError".to_owned(),
        status: Some(error.status_code),
        error_type: None,
        param: None,
        code: None,
        message: error.message,
    })
}

fn state(response: CompleteResponse) -> Value {
    let CompleteResponse {
        id,
        model,
        choices,
        usage,
        metadata: lists,
        provider_specific_fields,
        service_tier,
    } = response;
    let metadata = lists.map(|found| {
        json!({
            "grounding_metadata": found.grounding_metadata,
            "url_context_metadata": found.url_context_metadata,
            "safety_results": found.safety_results,
            "citation_metadata": found.citation_metadata,
        })
    });
    json!({
        "id": id,
        "model": model,
        "choices": choices,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
            "prompt_tokens_details": usage.prompt_tokens_details.is_some(),
            "completion_tokens_details": usage.completion_tokens_details.is_some(),
        },
        "metadata": metadata,
        "traffic_type": provider_specific_fields.map(|fields| fields.traffic_type),
        "service_tier": service_tier,
    })
}
