use std::fmt::Write as _;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::decode::decode_thread_message_reply;
use betterlite_openai::threads::error::{ReplyFault, ThreadsError};
use betterlite_openai::threads::records::ThreadRecord;
use http::header::{ACCEPT, AUTHORIZATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::Error as _;
use url::Url;

use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_thread_messages::error::ThreadMessagesError;
use crate::azure_thread_messages::operations::ThreadOperation;
use crate::azure_urls::url_parts::{encoded_query, first_query_values};
use crate::credentials::source::TokenReply;
use crate::endpoints::projection::TokenProvider;

const API_KEY: HeaderName = HeaderName::from_static("api-key");
const OPENAI_BETA: HeaderName = HeaderName::from_static("openai-beta");
// Python's quote(safe="!$&'()*+,;=:@") leaves these ASCII bytes bare inside a path parameter.
const KEPT_MARKS: &str = "!$&'()*+,;=:@-._~";

// The SDK's beta.threads calls: create posts the caller's fields and retrieve gets one thread by id.
pub(super) async fn send<T>(
    client: &AzureAssistantsClient<'_, T>,
    operation: ThreadOperation<'_>,
) -> Result<ThreadRecord, ThreadMessagesError>
where
    T: AssistantsTransport,
{
    let request = match operation {
        ThreadOperation::Create(data) => {
            let body =
                serde_json::to_vec(data).map_err(|source| ThreadsError::Encode { source })?;
            outbound(client, Method::POST, "threads", RequestBody::Json(body))?
        }
        ThreadOperation::Retrieve(thread_id) => {
            let tail = thread_tail(thread_id)?;
            outbound(client, Method::GET, &tail, RequestBody::Empty)?
        }
    };
    let response = client
        .transport
        .send(request)
        .await
        .map_err(|source| ThreadsError::Transport { source })?;
    if response.status >= 400 {
        return Err(ThreadMessagesError::Threads(ThreadsError::Status {
            status: response.status,
            body: response.body,
        }));
    }
    let thread = decode_thread_message_reply(&response.body)?;
    object_named_thread(thread)
}

fn outbound<T>(
    client: &AzureAssistantsClient<'_, T>,
    method: Method,
    tail: &str,
    body: RequestBody,
) -> Result<OutboundRequest, ThreadMessagesError> {
    let mut headers = auth_headers(client)?;
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(OPENAI_BETA, HeaderValue::from_static("assistants=v2"));
    Ok(OutboundRequest {
        method,
        url: target_url(&client.base_url, &client.api_version, tail)?,
        query: Vec::new(),
        headers,
        body,
        deadline: client.deadline,
    })
}

// The SDK joins the call path onto the raw base path and merges api-version into the base query.
fn target_url(base_url: &str, api_version: &str, tail: &str) -> Result<Url, ThreadMessagesError> {
    let joined = format!("{}/{tail}", base_url.trim_end_matches('/'));
    let mut url = Url::parse(&joined).map_err(|source| ThreadsError::BaseUrl { source })?;
    let query = versioned_query(url.query(), api_version);
    url.set_query(query.as_deref());
    Ok(url)
}

// api-version replaces the base URL's own pair in place, and an empty value leaves no query at all.
fn versioned_query(query: Option<&str>, api_version: &str) -> Option<String> {
    let mut pairs = first_query_values(query);
    let mut named = false;
    for (name, value) in &mut pairs {
        if name == "api-version" {
            api_version.clone_into(value);
            named = true;
        }
    }
    if !named && !api_version.is_empty() {
        pairs.push((String::from("api-version"), api_version.to_owned()));
    }
    pairs.retain(|(_, value)| !value.is_empty());
    encoded_query(&pairs)
}

// An empty id or a dot segment is refused before any request is built, as the SDK's path template does.
fn thread_tail(thread_id: &str) -> Result<String, ThreadMessagesError> {
    if thread_id.is_empty() {
        return Err(ThreadMessagesError::Threads(ThreadsError::EmptyThreadId));
    }
    if thread_id == "." || thread_id == ".." {
        return Err(ThreadMessagesError::Threads(ThreadsError::DotSegment {
            segment: thread_id.to_owned(),
        }));
    }
    Ok(format!("threads/{}", quote_thread_id(thread_id)))
}

fn quote_thread_id(thread_id: &str) -> String {
    let mut quoted = String::with_capacity(thread_id.len());
    for byte in thread_id.bytes() {
        if byte.is_ascii_alphanumeric() || KEPT_MARKS.as_bytes().contains(&byte) {
            quoted.push(char::from(byte));
        } else {
            let _encoded = write!(quoted, "%{byte:02X}");
        }
    }
    quoted
}

// A static or provider token is the bearer; the key rides along only when there is no static token.
fn auth_headers<T>(
    client: &AzureAssistantsClient<'_, T>,
) -> Result<HeaderMap, ThreadMessagesError> {
    let mut headers = HeaderMap::new();
    let mut bearer = client.token.clone();
    if bearer.is_none()
        && let Some(provider) = client
            .inputs
            .as_ref()
            .and_then(|inputs| inputs.token_provider.as_ref())
    {
        bearer = provider_token(provider)?;
    }
    if let Some(token) = bearer {
        headers.insert(
            AUTHORIZATION,
            sensitive(format!("Bearer {}", token.expose_secret()))?,
        );
    }
    let api_key = client
        .key
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty());
    if client.token.is_none()
        && let Some(key) = api_key
    {
        headers.insert(API_KEY, sensitive(key.expose_secret().to_owned())?);
    }
    Ok(headers)
}

// A provider that returns no text is refused, as the SDK's token getter refuses a non-string.
fn provider_token(
    provider: &TokenProvider<'_>,
) -> Result<Option<SecretString>, ThreadMessagesError> {
    match provider.0.token() {
        Ok(TokenReply::Text(token)) if !token.expose_secret().is_empty() => Ok(Some(token)),
        Ok(TokenReply::Text(_) | TokenReply::NonText { .. }) => {
            Err(ThreadMessagesError::Client(DefinitionsError::ProviderReply))
        }
        Err(failure) => Err(ThreadMessagesError::Client(DefinitionsError::Provider {
            class: failure.class,
            message: failure.message,
        })),
    }
}

fn sensitive(value: String) -> Result<HeaderValue, ThreadMessagesError> {
    let mut header =
        HeaderValue::try_from(value).map_err(|source| ThreadsError::Header { source })?;
    header.set_sensitive(true);
    Ok(header)
}

// LiteLLM's Thread declares object: Literal["thread"], so any other value fails its validation.
fn object_named_thread(thread: ThreadRecord) -> Result<ThreadRecord, ThreadMessagesError> {
    if thread.object == "thread" {
        return Ok(thread);
    }
    let source = serde_json::Error::custom("object must be \"thread\"");
    Err(ThreadMessagesError::Threads(ThreadsError::Reply {
        fault: ReplyFault::Shape,
        source,
    }))
}
