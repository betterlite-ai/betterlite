use std::time::Duration;

use betterlite_config::sections::openai_credentials::DEFAULT_API_BASE;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::{ACCEPT, AUTHORIZATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::ExposeSecret as _;
use url::Url;

use crate::assistants::client::SdkClient;
use crate::assistants::transport::AssistantsTransport;
use crate::thread_runs::error::ThreadRunsError;

const OPENAI_BETA: HeaderName = HeaderName::from_static("openai-beta");
const OPENAI_ORGANIZATION: HeaderName = HeaderName::from_static("openai-organization");
const POLL_HELPER: HeaderName = HeaderName::from_static("x-stainless-poll-helper");
const STREAM_HELPER: HeaderName = HeaderName::from_static("x-stainless-stream-helper");
const CUSTOM_HANDLER: HeaderName = HeaderName::from_static("x-stainless-custom-event-handler");
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
// The shared transport caps a response; the caller's timeout applies to each stream read instead.
const STREAM_DEADLINE: Duration = Duration::from_mins(100);
// The SDK quotes a path id with Python's quote(safe="!$&'()*+,;=:@"), so "/" is encoded and the unreserved marks stay.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=')
    .remove(b':')
    .remove(b'@');
// The SDK marks the poll and stream helpers on the request; the provider reads the custom-handler flag.
#[derive(Clone, Copy)]
pub(crate) enum CallKind {
    Create,
    Poll,
    Stream { custom_handler: bool },
}
pub(crate) fn outbound<T>(
    client: &SdkClient<T>,
    kind: CallKind,
    segments: &[&str],
    body: RequestBody,
) -> Result<OutboundRequest, ThreadRunsError> {
    let url = run_url(client, segments)?;
    let (method, headers) = call_headers(client, kind)?;
    Ok(OutboundRequest {
        method,
        url,
        query: Vec::new(),
        headers,
        body,
        deadline: match kind {
            CallKind::Stream { .. } => STREAM_DEADLINE,
            CallKind::Create | CallKind::Poll => client.timeout.unwrap_or(DEFAULT_TIMEOUT),
        },
    })
}
fn run_url<T>(client: &SdkClient<T>, segments: &[&str]) -> Result<Url, ThreadRunsError> {
    let mut url = Url::parse(client.base_url.as_deref().unwrap_or(DEFAULT_API_BASE))
        .map_err(ThreadRunsError::BaseUrl)?;
    let path = format!("{}/", url.path().trim_end_matches('/'));
    let quoted: Vec<String> = segments
        .iter()
        .map(|segment| utf8_percent_encode(segment, PATH_SEGMENT).to_string())
        .collect();
    url.set_path(&format!("{path}{}", quoted.join("/")));
    Ok(url)
}
fn call_headers<T>(
    client: &SdkClient<T>,
    kind: CallKind,
) -> Result<(Method, HeaderMap), ThreadRunsError> {
    let api_key = client
        .api_key
        .as_ref()
        .ok_or(ThreadRunsError::MissingApiKey)?;
    let mut headers = HeaderMap::new();
    let mut bearer = HeaderValue::try_from(format!("Bearer {}", api_key.expose_secret()))
        .map_err(ThreadRunsError::Header)?;
    bearer.set_sensitive(true);
    headers.insert(AUTHORIZATION, bearer);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(OPENAI_BETA, HeaderValue::from_static("assistants=v2"));
    if let Some(organization) = &client.organization {
        let value =
            HeaderValue::try_from(organization.as_str()).map_err(ThreadRunsError::Header)?;
        headers.insert(OPENAI_ORGANIZATION, value);
    }
    let method = match kind {
        CallKind::Create | CallKind::Stream { .. } => Method::POST,
        CallKind::Poll => {
            headers.insert(POLL_HELPER, HeaderValue::from_static("true"));
            Method::GET
        }
    };
    if let CallKind::Stream { custom_handler } = kind {
        headers.insert(
            STREAM_HELPER,
            HeaderValue::from_static("threads.runs.create_and_stream"),
        );
        let flag = if custom_handler { "true" } else { "false" };
        headers.insert(CUSTOM_HANDLER, HeaderValue::from_static(flag));
    }
    Ok((method, headers))
}
pub(crate) async fn send<T>(
    transport: &T,
    request: OutboundRequest,
) -> Result<UpstreamResponse, ThreadRunsError>
where
    T: AssistantsTransport,
{
    let response = transport
        .send(request)
        .await
        .map_err(ThreadRunsError::Transport)?;
    if response.status >= 400 {
        return Err(ThreadRunsError::Status(response.status, response.body));
    }
    Ok(response)
}
