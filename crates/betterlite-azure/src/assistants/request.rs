use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::{RunReply, RunRequest};
use http::header::{ACCEPT, AUTHORIZATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::error::Category;
use url::Url;

use crate::assistants::error::AssistantsRunError;
use crate::assistants::reply;
use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_urls::url_parts::{encoded_query, first_query_values};
use crate::credentials::source::TokenReply;
use crate::endpoints::projection::TokenProvider;

const API_KEY: HeaderName = HeaderName::from_static("api-key");
const OPENAI_BETA: HeaderName = HeaderName::from_static("openai-beta");
// Python's quote(safe="!$&'()*+,;=:@") keeps these marks, and the unreserved marks, bare in a path parameter.
const SEGMENT_ENCODE: AsciiSet = NON_ALPHANUMERIC
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
    .remove(b'@')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

// The SDK sends every keyword LiteLLM's run wrapper received, so a None argument stays null on the wire.
#[derive(Serialize)]
struct RunBody<'call> {
    assistant_id: &'call str,
    additional_instructions: Option<&'call str>,
    instructions: Option<&'call str>,
    metadata: Option<&'call JsonValue>,
    model: Option<&'call str>,
    stream: bool,
    tools: Option<&'call JsonValue>,
}

pub(super) fn create_body(run: &RunRequest, stream: bool) -> Result<Vec<u8>, AssistantsRunError> {
    let body = RunBody {
        assistant_id: run.assistant_id.as_str(),
        additional_instructions: run.additional_instructions.as_deref(),
        instructions: run.instructions.as_deref(),
        metadata: run.metadata.as_ref(),
        model: run.model.as_deref(),
        stream,
        tools: run.tools.as_ref(),
    };
    serde_json::to_vec(&body).map_err(AssistantsRunError::Encode)
}

pub(super) fn run_tail(thread_id: &str) -> Result<String, AssistantsRunError> {
    Ok(format!(
        "threads/{}/runs",
        quoted(thread_id, ThreadRunsError::EmptyThreadId)?
    ))
}

pub(super) fn poll_tail(thread_id: &str, run_id: &str) -> Result<String, AssistantsRunError> {
    let thread = quoted(thread_id, ThreadRunsError::EmptyThreadId)?;
    let run = quoted(run_id, ThreadRunsError::EmptyRunId)?;
    Ok(format!("threads/{thread}/runs/{run}"))
}

// An empty id or a dot segment is refused before any request is built, as the SDK's path template does.
fn quoted(segment: &str, empty: ThreadRunsError) -> Result<String, AssistantsRunError> {
    if segment.is_empty() {
        return Err(AssistantsRunError::Runs(empty));
    }
    if segment == "." || segment == ".." {
        return Err(AssistantsRunError::Runs(ThreadRunsError::DotSegment(
            segment.to_owned(),
        )));
    }
    Ok(utf8_percent_encode(segment, &SEGMENT_ENCODE).to_string())
}

pub(super) fn outbound<T>(
    client: &AzureAssistantsClient<'_, T>,
    method: Method,
    tail: &str,
    body: RequestBody,
    deadline: Duration,
) -> Result<OutboundRequest, AssistantsRunError> {
    let mut headers = auth_headers(client)?;
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(OPENAI_BETA, HeaderValue::from_static("assistants=v2"));
    Ok(OutboundRequest {
        method,
        url: target(client, tail)?,
        query: Vec::new(),
        headers,
        body,
        deadline,
    })
}

// The SDK joins the tail onto the raw base path and query with one slash, and keeps a fragment at the end.
fn target<T>(client: &AzureAssistantsClient<'_, T>, tail: &str) -> Result<Url, AssistantsRunError> {
    let at = client.base_url.find('#').unwrap_or(client.base_url.len());
    let (head, fragment) = client.base_url.split_at(at);
    let slash = if head.ends_with('/') { "" } else { "/" };
    let joined = format!("{head}{slash}{tail}{fragment}");
    let mut url = Url::parse(&joined)
        .map_err(|source| AssistantsRunError::Runs(ThreadRunsError::BaseUrl(source)))?;
    let query = merge_version(url.query(), client.api_version.as_str());
    url.set_query(query.as_deref());
    Ok(url)
}

fn merge_version(query: Option<&str>, api_version: &str) -> Option<String> {
    let mut merged: Vec<(String, String)> = Vec::new();
    let mut named = false;
    for (name, value) in first_query_values(query) {
        if name == "api-version" {
            named = true;
            merged.push((name, api_version.to_owned()));
        } else {
            merged.push((name, value));
        }
    }
    if !named {
        merged.push((String::from("api-version"), api_version.to_owned()));
    }
    // The SDK's query builder drops a parameter whose value is empty.
    merged.retain(|(_, value)| !value.is_empty());
    encoded_query(&merged)
}

// A decoded run reply keeps the response headers, since the poll helper reads its interval from them.
pub(super) struct Fetched {
    pub(super) reply: RunReply,
    pub(super) headers: HeaderMap,
}

pub(super) async fn fetch<T>(
    client: &AzureAssistantsClient<'_, T>,
    request: OutboundRequest,
) -> Result<Fetched, AssistantsRunError>
where
    T: AssistantsTransport,
{
    let response: UpstreamResponse = client
        .transport
        .send(request)
        .await
        .map_err(ThreadRunsError::Transport)?;
    if response.status >= 400 {
        return Err(AssistantsRunError::Runs(ThreadRunsError::Status(
            response.status,
            response.body,
        )));
    }
    let mut value = serde_json::from_slice::<serde_json::Value>(&response.body)
        .map_err(|source| AssistantsRunError::Runs(classify(source)))?;
    reply::sdk_shape(&mut value);
    let reply = serde_json::from_value(value)
        .map_err(|source| AssistantsRunError::Runs(classify(source)))?;
    Ok(Fetched {
        reply,
        headers: response.headers,
    })
}

fn classify(source: serde_json::Error) -> ThreadRunsError {
    match source.classify() {
        Category::Eof => ThreadRunsError::Truncated(source),
        Category::Data => ThreadRunsError::Shape(source),
        Category::Syntax | Category::Io => ThreadRunsError::Malformed(source),
    }
}

// With a token the SDK sends the bearer alone; the key rides along only when no token source answers.
fn auth_headers<T>(client: &AzureAssistantsClient<'_, T>) -> Result<HeaderMap, AssistantsRunError> {
    let mut headers = HeaderMap::new();
    let token = match client.token.clone() {
        Some(token) => Some(token),
        None => client
            .inputs
            .as_ref()
            .and_then(|inputs| inputs.token_provider.as_ref())
            .map(provider_token)
            .transpose()?,
    };
    if let Some(token) = token {
        headers.insert(
            AUTHORIZATION,
            sensitive(format!("Bearer {}", token.expose_secret()))?,
        );
    } else if let Some(key) = client
        .key
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty())
    {
        headers.insert(API_KEY, sensitive(key.expose_secret().to_owned())?);
    }
    Ok(headers)
}

// A provider that returns no text is refused, as the SDK's token getter refuses a non-string.
fn provider_token(provider: &TokenProvider<'_>) -> Result<SecretString, AssistantsRunError> {
    match provider.0.token() {
        Ok(TokenReply::Text(token)) if !token.expose_secret().is_empty() => Ok(token),
        Ok(TokenReply::Text(_) | TokenReply::NonText { .. }) => {
            Err(AssistantsRunError::Client(DefinitionsError::ProviderReply))
        }
        Err(failure) => Err(AssistantsRunError::Client(DefinitionsError::Provider {
            class: failure.class,
            message: failure.message,
        })),
    }
}

fn sensitive(value: String) -> Result<HeaderValue, AssistantsRunError> {
    let mut header = HeaderValue::try_from(value)
        .map_err(|source| AssistantsRunError::Runs(ThreadRunsError::Header(source)))?;
    header.set_sensitive(true);
    Ok(header)
}
