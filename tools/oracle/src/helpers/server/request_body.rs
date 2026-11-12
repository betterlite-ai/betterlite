use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::request_body::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{
    JsonObject, MediaTypeKind, ParsedBody, RequestBodyInput, classify_media_type,
    is_opaque_audio_request, is_otlp_trace_request, normalize_media_type, parse_binary_body,
    rewrite_request_model,
};
use betterlite_server::api::request_body::read_request_body;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    NormalizeMediaTypes {
        content_types: Vec<String>,
    },
    ParseBinaryBody {
        body: WireBody,
    },
    IsOtlpTraceRequest {
        method: String,
        route_path: String,
    },
    IsOpaqueAudioRequest {
        route: String,
        content_type: String,
    },
    ReadRequestBody {
        request: WireRequest,
        max_repair_bytes: Option<u64>,
    },
    CachedBody {
        request: WireRequest,
    },
    SetCachedBody {
        request: WireRequest,
        body: JsonObject,
    },
    RewriteRequestModel {
        request_data: JsonObject,
        model: String,
        request: Option<WireRequest>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireRequest {
    method: String,
    route_path: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    query_params: BTreeMap<String, String>,
    body: WireBody,
    #[serde(default)]
    cached_body: Option<JsonObject>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireBody {
    encoding: WireEncoding,
    value: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireEncoding {
    Utf8,
    Base64,
}

#[derive(Serialize)]
struct MediaTypeFacts<'kind> {
    media_type: String,
    kind: &'kind str,
}

#[derive(Serialize)]
struct ReadFacts {
    body: JsonObject,
    body_read: bool,
    cached: Option<JsonObject>,
    raw: Option<String>,
}

#[derive(Serialize)]
struct RewriteFacts {
    request_data: JsonObject,
    cached: Option<JsonObject>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::NormalizeMediaTypes { content_types } => {
                Ok(serde_json::to_value(media_facts(content_types))?)
            }
            Input::ParseBinaryBody { body } => {
                Ok(serde_json::to_value(parse_binary_body(&body.decode()?))?)
            }
            Input::IsOtlpTraceRequest { method, route_path } => Ok(serde_json::to_value(
                is_otlp_trace_request(method, route_path),
            )?),
            Input::IsOpaqueAudioRequest {
                route,
                content_type,
            } => Ok(serde_json::to_value(is_opaque_audio_request(
                route,
                content_type,
            ))?),
            Input::ReadRequestBody {
                request,
                max_repair_bytes,
            } => read_facts(request, *max_repair_bytes),
            Input::CachedBody { request } => {
                let facts = request.clone().into_input()?;
                Ok(serde_json::to_value(
                    facts.cached_body.as_ref().map(ParsedBody::visible),
                )?)
            }
            Input::SetCachedBody { request, body } => {
                let mut facts = request.clone().into_input()?;
                facts.cached_body = Some(ParsedBody::from_client_body(body.clone()));
                Ok(serde_json::to_value(
                    facts.cached_body.as_ref().map(ParsedBody::visible),
                )?)
            }
            Input::RewriteRequestModel {
                request_data,
                model,
                request,
            } => rewrite_facts(request_data, model, request.as_ref()),
        }
    }
}

fn media_facts(content_types: &[String]) -> Vec<MediaTypeFacts<'_>> {
    content_types
        .iter()
        .map(|content_type| MediaTypeFacts {
            media_type: normalize_media_type(content_type),
            kind: media_kind(content_type),
        })
        .collect()
}

fn media_kind(content_type: &str) -> &'static str {
    match classify_media_type(content_type) {
        MediaTypeKind::Json => "json",
        MediaTypeKind::Form => "form",
        MediaTypeKind::Binary => "binary",
        MediaTypeKind::Other => "other",
    }
}

fn read_facts(request: &WireRequest, max_repair_bytes: Option<u64>) -> HelperResult {
    let facts = request.clone().into_input()?;
    let settings = Settings { max_repair_bytes };
    match read_request_body(&facts, &settings, &ApiLimits::default()) {
        Ok(result) => {
            let cached = result.cached_body();
            let raw = result
                .raw_json_body(&facts)
                .map(|bytes| STANDARD.encode(bytes));
            Ok(serde_json::to_value(ReadFacts {
                body: result.body,
                body_read: result.body_read,
                cached,
                raw,
            })?)
        }
        Err(error) => Ok(serde_json::to_value(error.facts())?),
    }
}

fn rewrite_facts(
    request_data: &JsonObject,
    model: &str,
    request: Option<&WireRequest>,
) -> HelperResult {
    let input = match request {
        Some(wire) => Some(wire.clone().into_input()?),
        None => None,
    };
    let rewritten = rewrite_request_model(request_data, input.as_ref(), model);
    Ok(serde_json::to_value(RewriteFacts {
        request_data: rewritten.request_data,
        cached: rewritten.cached_body.as_ref().map(ParsedBody::visible),
    })?)
}

impl WireRequest {
    fn into_input(self) -> Result<RequestBodyInput, Box<HelperError>> {
        let body = self.body.decode()?;
        Ok(RequestBodyInput {
            method: self.method,
            route_path: self.route_path,
            headers: self.headers,
            query_params: self.query_params,
            body,
            client_requested_model: None,
            cached_body: self.cached_body.map(ParsedBody::from_client_body),
            form_fields: None,
        })
    }
}

impl WireBody {
    fn decode(&self) -> Result<Vec<u8>, Box<HelperError>> {
        match self.encoding {
            WireEncoding::Utf8 => Ok(self.value.as_bytes().to_vec()),
            WireEncoding::Base64 => STANDARD.decode(&self.value).map_err(|error| {
                Box::new(HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            }),
        }
    }
}
