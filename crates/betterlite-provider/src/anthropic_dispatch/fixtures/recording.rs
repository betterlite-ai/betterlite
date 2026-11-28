use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::batches::handler::BatchTransport;
use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::decisions::Json;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use http::HeaderMap;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use super::super::base::Surface;
use super::super::completion::{CompletionHandler, HandlerFuture, ResolvedCall};
use super::{FixtureError, Input, Recorded};

pub(super) fn json<T>(value: &T) -> Result<Json, FixtureError>
where
    T: Serialize,
{
    let encoded = serde_json::to_vec(value)?;
    Ok(serde_json::from_slice(&encoded)?)
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Failure {
    #[error("scripted completion failed with status {0}")]
    Status(u16),
    #[error("completion capture failed")]
    Capture(#[from] FixtureError),
}

pub(super) struct Recorder<'input>(pub(super) &'input Input);
impl CompletionHandler<String, String> for Recorder<'_> {
    type Reply = Json;
    type Failure = Failure;

    fn complete<'call>(
        &'call mut self,
        call: ResolvedCall<String, String>,
        _log: &'call mut LogContext<'_, '_>,
    ) -> HandlerFuture<'call, Json, Failure> {
        Box::pin(async move {
            if let Some(status) = self.0.failure {
                return Err(Failure::Status(status));
            }
            let reply = Capture::from((call, self.0));
            Ok(json(&reply)?)
        })
    }
    fn logged_response(reply: &Json) -> Result<Json, Failure> {
        Ok(reply.clone())
    }
}

#[derive(Serialize)]
struct Capture {
    model: String,
    provider: String,
    model_response_id: String,
    timeout_ms: u128,
    api_base: Option<String>,
    auth_matches: bool,
    messages: Vec<Json>,
    headers: BTreeMap<String, String>,
    optional_params: indexmap::IndexMap<String, Json>,
    litellm_params: indexmap::IndexMap<String, Json>,
    custom_prompt_dict: Option<BTreeMap<String, Json>>,
    stream: Option<bool>,
    acompletion: bool,
    client: Option<String>,
    shared_session: Option<String>,
    encoding: String,
    config: Option<String>,
}

impl From<(ResolvedCall<String, String>, &Input)> for Capture {
    #[rustfmt::skip]
    fn from((resolved, input): (ResolvedCall<String, String>, &Input)) -> Self {
        let call = resolved.call;
        Self {
            model: call.model, provider: call.provider, api_base: call.api_base,
            model_response_id: call.model_response.id, timeout_ms: call.timeout.as_millis(),
            auth_matches: call.api_key.as_ref().map(ExposeSecret::expose_secret) == input.expected_key.as_ref().map(ExposeSecret::expose_secret),
            messages: call.messages, headers: call.headers.into_iter().map(|(name, value)| (name, value.expose_secret().to_owned())).collect(), optional_params: call.optional_params, litellm_params: call.litellm_params,
            custom_prompt_dict: match resolved.surface { Surface::Messages => Some(call.custom_prompt_dict), Surface::Legacy => None },
            stream: match resolved.surface { Surface::Messages => None, Surface::Legacy => Some(call.stream) },
            acompletion: call.acompletion, client: call.client, shared_session: call.shared_session,
            encoding: call.encoding, config: resolved.config.map(|tag| tag.class_name().to_owned()),
        }
    }
}

#[derive(Clone)]
pub(super) struct Transport {
    pub(super) sent: Arc<Mutex<Option<Recorded>>>,
    pub(super) response: String,
    pub(super) expected_key: Option<SecretString>,
}

impl Transport {
    fn answer(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let headers = self.headers(&request.headers);
        let body = body(request.body)?;
        *self.sent.lock().unwrap_or_else(PoisonError::into_inner) = Some(Recorded {
            method: request.method.to_string(),
            url: request.url.to_string(),
            query: request.query,
            headers,
            body,
        });
        Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: self.response.as_bytes().to_vec(),
        })
    }
    fn headers(&self, headers: &HeaderMap) -> BTreeMap<String, String> {
        headers
            .iter()
            .map(|(name, value)| {
                let matched = if value.to_str().ok()
                    == self.expected_key.as_ref().map(ExposeSecret::expose_secret)
                {
                    "matched"
                } else {
                    "mismatch"
                };
                let shown = if name == "x-api-key" || name == "authorization" {
                    matched
                } else {
                    value.to_str().unwrap_or("invalid")
                };
                (name.to_string(), shown.to_owned())
            })
            .collect()
    }
}

fn body(request: RequestBody) -> Result<Json, TransportError> {
    let value = match request {
        RequestBody::Empty => Json::Null,
        RequestBody::Json(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_error| TransportError::Closed)?
        }
        RequestBody::Bytes { data, .. } => {
            Json::Text(String::from_utf8(data).map_err(|_error| TransportError::Closed)?)
        }
        RequestBody::Multipart { boundary, parts } => {
            let recorded_parts: Vec<_> = parts
                .into_iter()
                .map(|part| {
                    Ok(Part {
                        name: part.name,
                        filename: part.file_name,
                        content_type: part.content_type,
                        data: String::from_utf8(part.data)
                            .map_err(|_error| TransportError::Closed)?,
                    })
                })
                .collect::<Result<Vec<_>, TransportError>>()?;
            json(&Multipart {
                boundary,
                parts: recorded_parts,
            })
            .map_err(|_error| TransportError::Closed)?
        }
    };
    Ok(value)
}

#[derive(Serialize)]
struct Multipart {
    boundary: String,
    parts: Vec<Part>,
}
#[derive(Serialize)]
struct Part {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    data: String,
}

impl CountTokensTransport for Transport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.answer(request))
    }
}
impl BatchTransport for Transport {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.answer(request))
    }
}
impl FileTransport for Transport {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(self.answer(request))
    }
    fn send_stream(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        ready(Err(TransportError::Closed))
    }
}
