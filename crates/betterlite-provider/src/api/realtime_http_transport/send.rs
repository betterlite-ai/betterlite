use std::time::{Duration, Instant};

use betterlite_core::api::realtime_contracts::config::BaseRealtimeHttpConfig;
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::vector_store_http_write::error::{FailureFacts, ResponseFacts};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use serde_json::Number;
use url::Url;

use super::error::{
    RealtimeHttpError, StatusFailure, TimeoutFailure, display_facts, provider_failure,
    status_failure, timeout_failure,
};

pub(crate) struct WireRequest<'wire> {
    pub(crate) url: &'wire str,
    pub(crate) headers: &'wire HeaderMap,
    pub(crate) body: RequestBody,
    pub(crate) timeout: Option<Duration>,
}

// One POST, no retry: a transport failure or a status of 400 or more becomes the provider's error when a config is present
pub(crate) async fn send<C>(
    config: Option<&C>,
    client: &ProxiedClient,
    request: WireRequest<'_>,
) -> Result<UpstreamResponse, RealtimeHttpError>
where
    C: BaseRealtimeHttpConfig,
{
    let deadline = request.timeout.unwrap_or(client.limits.read_timeout);
    let built = build(request, deadline)?;
    let started = Instant::now();
    let reply = match client.send_following(built).await {
        Ok(response) => response.buffered(client.limits.body_bytes).await,
        Err(error) => Err(error),
    };
    let elapsed = started.elapsed();
    match reply {
        Ok(answer) if answer.status < 400 => Ok(answer),
        Ok(answer) => Err(rejected(config, &answer)),
        Err(error) => Err(failed(config, error, deadline, elapsed)),
    }
}

// A URL or header HTTP cannot carry fails here, before the send catch and before any request leaves the process
fn build(
    request: WireRequest<'_>,
    deadline: Duration,
) -> Result<OutboundRequest, RealtimeHttpError> {
    let url = Url::parse(request.url).map_err(|source| RealtimeHttpError::RequestUrl {
        url: request.url.to_owned(),
        source,
    })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: wire_headers(request.headers)?,
        body: request.body,
        deadline,
    })
}

fn wire_headers(headers: &HeaderMap) -> Result<HttpHeaders, RealtimeHttpError> {
    let mut wire = HttpHeaders::new();
    for (name, value) in headers {
        let Json::Text(text) = value else {
            return Err(RealtimeHttpError::HeaderNotText { name: name.clone() });
        };
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            RealtimeHttpError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(text).map_err(|source| RealtimeHttpError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        wire.insert(header_name, header_value);
    }
    Ok(wire)
}

fn rejected<C>(config: Option<&C>, answer: &UpstreamResponse) -> RealtimeHttpError
where
    C: BaseRealtimeHttpConfig,
{
    let StatusFailure {
        status_code,
        message,
        headers,
    } = status_failure(answer);
    let Some(provider) = config else {
        return RealtimeHttpError::Status(StatusFailure {
            status_code,
            message,
            headers,
        });
    };
    let facts = FailureFacts {
        response_status: Some(i64::from(status_code)),
        status: Some(Json::Number(Number::from(status_code))),
        text: Some(message.clone()),
        headers: None,
        response: Some(ResponseFacts {
            status_code: i64::from(status_code),
            text: Some(message),
            headers: Some(headers),
        }),
        display: String::new(),
    };
    RealtimeHttpError::Provider(provider_failure(provider, &facts))
}

fn failed<C>(
    config: Option<&C>,
    error: TransportError,
    timeout: Duration,
    elapsed: Duration,
) -> RealtimeHttpError
where
    C: BaseRealtimeHttpConfig,
{
    let timed_out = matches!(
        &error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    );
    if !timed_out {
        let Some(provider) = config else {
            return RealtimeHttpError::Transport(error);
        };
        return RealtimeHttpError::Provider(provider_failure(
            provider,
            &display_facts(error.to_string()),
        ));
    }
    let TimeoutFailure { message } = timeout_failure(timeout, elapsed);
    let Some(provider) = config else {
        return RealtimeHttpError::Timeout(TimeoutFailure { message });
    };
    let facts = FailureFacts {
        response_status: None,
        status: Some(Json::Number(Number::from(408_u16))),
        text: Some(message),
        headers: Some(HeaderMap::new()),
        response: None,
        display: String::new(),
    };
    RealtimeHttpError::Provider(provider_failure(provider, &facts))
}
