use std::time::Duration;

use betterlite_core::api::responses_stream_driver::errors::{TransportFailure, TransportKind};
use betterlite_core::api::responses_stream_driver::source::{SourceFailure, classify};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::stream::SseParser;
use futures_util::StreamExt as _;
use http::header::{AUTHORIZATION, HeaderValue};
use http::{HeaderMap, Method};
use serde::Serialize;
use url::Url;

use super::call::{Call, FailureKind, RunSpec, ScriptedFailure};
use crate::sdk_http::error::SdkHttpError;

pub(super) type Opened = Vec<Result<Vec<u8>, SourceFailure>>;

#[derive(Serialize)]
struct Wire<'wire> {
    model: &'wire str,
    input: &'wire betterlite_models::decisions::Json,
    stream: bool,
}

fn failure(spec: &ScriptedFailure) -> TransportFailure {
    let kind = match spec.kind {
        FailureKind::Read => TransportKind::ReadError,
        FailureKind::Protocol => TransportKind::RemoteProtocolError,
        FailureKind::Timeout => TransportKind::HttpError,
    };
    TransportFailure::new(kind, &spec.message)
}

// A scripted run never touches the provider: the payloads and the read failure are case data
pub(super) fn scripted(spec: &RunSpec) -> Opened {
    let mut items: Opened = spec
        .payloads
        .iter()
        .map(|text| Ok(text.as_bytes().to_vec()))
        .collect();
    if let Some(failure) = &spec.failure {
        items.push(Err(SourceFailure::Transport(self::failure(failure))));
    }
    items
}

fn headers(call: &Call) -> Result<HeaderMap, SdkHttpError> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_bytes(format!("Bearer {}", call.api_key).as_bytes())
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

// One prepared POST per live run, framed into the payloads both iterators then read
pub(super) async fn prefetch(
    client: &ProxiedClient,
    call: &Call,
    api_base: &str,
) -> Result<Opened, SdkHttpError> {
    let body = serde_json::to_vec(&Wire {
        model: &call.model,
        input: &call.input,
        stream: true,
    })
    .map_err(SdkHttpError::Serialize)?;
    let url = format!("{}/responses", api_base.trim_end_matches('/'));
    let response = client
        .send(OutboundRequest {
            method: Method::POST,
            url: Url::parse(&url).map_err(|source| SdkHttpError::Url(Box::new(source)))?,
            query: Vec::new(),
            headers: headers(call)?,
            body: RequestBody::Json(body),
            deadline: Duration::from_secs(call.timeout_seconds),
        })
        .await
        .map_err(|source| SdkHttpError::Transport(Box::new(source)))?;
    let status = response.response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(SdkHttpError::Invalid(format!(
            "the mock answered the responses call with status {status}"
        )));
    }
    let mut body = response.body_stream();
    let mut parser = SseParser::default();
    let mut items = Opened::new();
    while let Some(chunk) = body.next().await {
        let bytes = match chunk {
            Ok(bytes) => bytes,
            Err(error) => {
                items.push(Err(SourceFailure::Transport(classify(error))));
                return Ok(items);
            }
        };
        match parser.push(&bytes) {
            Ok(payloads) => items.extend(payloads.into_iter().map(Ok)),
            Err(error) => {
                items.push(Err(SourceFailure::Decode(error.to_string())));
                return Ok(items);
            }
        }
    }
    items.extend(parser.finish().into_iter().map(Ok));
    Ok(items)
}
