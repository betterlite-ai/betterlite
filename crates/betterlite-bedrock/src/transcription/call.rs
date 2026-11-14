use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use aws_smithy_async::time::SharedTimeSource;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::transcription::transport::TranscriptionTransport;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use super::auth::{
    ChainCredentials, resolve_credentials, sign_transcription, validate_environment,
};
use super::binding::{BindingArguments, TranscriptionBinding};
use super::error::{CallError, TranscriptionError};
use super::params::{Params, json_type_name};
use super::reply::decode_transcription_reply;
use super::request::transcription_body;
use super::url::complete_url;

// LiteLLM's native handler waits ten minutes when the caller sets no usable timeout
const DEFAULT_TIMEOUT: Duration = Duration::from_mins(10);
const ERROR_BODY_CHARS: usize = 256;
const SECRET_HEADERS: [&str; 2] = ["x-amz-security-token", "authorization"];

pub struct NativeTranscription<T, C> {
    pub transport: T,
    pub credentials: C,
    pub environment: BTreeMap<String, String>,
    pub clock: SharedTimeSource,
}

fn string_headers(extra: Option<&Params>) -> Result<Vec<(String, String)>, CallError> {
    extra
        .into_iter()
        .flatten()
        .map(|(name, value)| {
            let Json::Text(text) = value else {
                return Err(CallError::HeaderType {
                    name: name.clone(),
                    actual: json_type_name(value),
                });
            };
            Ok((name.clone(), text.clone()))
        })
        .collect()
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, CallError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let (Ok(header_name), Ok(mut header_value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) else {
            return Err(CallError::InvalidHeader { name: name.clone() });
        };
        header_value.set_sensitive(SECRET_HEADERS.contains(&header_name.as_str()));
        map.append(header_name, header_value);
    }
    Ok(map)
}

// LiteLLM reads response.text, which decodes any bytes; betterlite refuses bytes that are not UTF-8
fn error_body(body: &[u8]) -> String {
    let text = String::from_utf8(body.to_vec())
        .unwrap_or_else(|error| format!("{} bytes that are not UTF-8", error.as_bytes().len()));
    let kept: String = text.chars().take(ERROR_BODY_CHARS).collect();
    if text.chars().count() > ERROR_BODY_CHARS {
        format!("{kept}... (truncated)")
    } else {
        kept
    }
}

impl<T, C> NativeTranscription<T, C>
where
    T: TranscriptionTransport,
    C: ChainCredentials,
{
    async fn signed_request(
        &self,
        arguments: &BindingArguments<'_>,
    ) -> Result<OutboundRequest, CallError> {
        let call = arguments.call;
        let validated = validate_environment(
            string_headers(call.extra_headers)?,
            call.model,
            call.optional_params,
            &self.environment,
        );
        let url = complete_url(
            call.api_base,
            call.model,
            call.optional_params,
            &self.environment,
        );
        let audio = serde_json::to_value(&arguments.audio)
            .and_then(serde_json::from_value::<Json>)
            .map_err(TranscriptionError::Encode)?;
        let body = transcription_body(&audio, call.optional_params)?;
        let credentials = resolve_credentials(&validated.credentials, &self.credentials).await?;
        let signed = sign_transcription(&validated, &url, &body, &credentials, self.clock.now())?;
        // the native optional_timeout counts a zero, negative, NaN or infinite timeout as unset
        let deadline = match arguments.timeout_seconds {
            Some(seconds) if seconds.is_finite() && seconds > 0.0_f64 => {
                Duration::try_from_secs_f64(seconds)
                    .map_err(|source| CallError::Timeout { seconds, source })?
            }
            Some(_) | None => DEFAULT_TIMEOUT,
        };
        Ok(OutboundRequest {
            method: Method::POST,
            url: Url::parse(&signed.url).map_err(CallError::Endpoint)?,
            query: Vec::new(),
            headers: header_map(&signed.headers)?,
            body: RequestBody::Json(signed.body),
            deadline,
        })
    }

    async fn send(&self, arguments: &BindingArguments<'_>) -> Result<Vec<u8>, CallError> {
        let outbound = self.signed_request(arguments).await?;
        let response = self
            .transport
            .send(outbound)
            .await
            .map_err(CallError::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(CallError::Status {
                status: response.status,
                body: error_body(&response.body),
            });
        }
        Ok(response.body)
    }
}

#[async_trait]
impl<T, C> TranscriptionBinding for NativeTranscription<T, C>
where
    T: TranscriptionTransport,
    C: ChainCredentials,
{
    type Error = CallError;

    fn transcribe(&self, _arguments: &BindingArguments<'_>) -> Result<Params, CallError> {
        Err(CallError::Blocking)
    }

    async fn atranscribe(&self, arguments: &BindingArguments<'_>) -> Result<Params, CallError> {
        let reply = decode_transcription_reply(&self.send(arguments).await?)?;
        Ok(Params::from_iter([(
            "text".to_owned(),
            Json::Text(reply.text),
        )]))
    }
}
