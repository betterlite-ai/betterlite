use std::future::Future;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use url::Url;

use crate::files::error::StatusError;
use crate::files::stream::read_error_body;
use crate::speech::client::{ClientKind, SpeechClient, SpeechConnection, select_client};
use crate::speech::error::SpeechError;
use crate::speech::logging::{SpeechLogRecord, SpeechLogger};
use crate::speech::request::{SpeechRequest, logged_body, wire_body};
use crate::speech::result::{SpeechResponse, SpeechTransport};

pub struct SpeechCall<'call> {
    pub request: SpeechRequest<'call>,
    pub connection: SpeechConnection,
    pub aspeech: Option<bool>,
}

fn speech_url(client: &SpeechClient) -> Result<Url, SpeechError> {
    client
        .base_url
        .join("audio/speech")
        .map_err(|source| SpeechError::BaseUrl { source })
}

fn outbound_request(
    client: &SpeechClient,
    request: &SpeechRequest<'_>,
) -> Result<OutboundRequest, SpeechError> {
    let api_key = client.api_key.as_ref().ok_or(SpeechError::MissingApiKey)?;
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/octet-stream"));
    let mut authorization = HeaderValue::try_from(format!("Bearer {}", api_key.expose_secret()))
        .map_err(|source| SpeechError::Header {
            source: http::Error::from(source),
        })?;
    authorization.set_sensitive(true);
    headers.insert(AUTHORIZATION, authorization);
    for (name, value) in &request.options.extra_headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| SpeechError::Header {
                source: http::Error::from(source),
            })?;
        let header_value = HeaderValue::from_str(value).map_err(|source| SpeechError::Header {
            source: http::Error::from(source),
        })?;
        headers.insert(header_name, header_value);
    }
    Ok(OutboundRequest {
        method: Method::POST,
        url: speech_url(client)?,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(wire_body(request)?),
        deadline: client.timeout,
    })
}

// LiteLLM's audio_speech plus async_audio_speech: aspeech is True picks the async client, every other value the sync one.
pub fn audio_speech<'call, T>(
    call: SpeechCall<'_>,
    log: &mut dyn SpeechLogger,
    transport: &'call T,
) -> Result<impl Future<Output = Result<SpeechResponse, SpeechError>> + Send + 'call, SpeechError>
where
    T: SpeechTransport,
{
    let SpeechCall {
        request,
        connection,
        aspeech,
    } = call;
    let kind = if aspeech == Some(true) {
        ClientKind::Async
    } else {
        ClientKind::Sync
    };
    let client = select_client(&connection, kind)?;
    let outbound = outbound_request(&client, &request)?;
    let body = logged_body(&request)?;
    log.pre_call(SpeechLogRecord {
        input: request.input,
        api_base: client.base_url.as_str(),
        body,
    })
    .map_err(|source| SpeechError::Logging { source })?;
    Ok(async move {
        let reply = transport
            .send(outbound)
            .await
            .map_err(|source| SpeechError::Transport { source })?;
        if !(200..300).contains(&reply.status) {
            let error_body = read_error_body(reply.body).await;
            return Err(SpeechError::Status(Box::new(StatusError::from_response(
                reply.status,
                &error_body,
            ))));
        }
        Ok(SpeechResponse::from(reply))
    })
}
