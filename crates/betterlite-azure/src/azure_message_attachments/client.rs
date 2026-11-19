use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::decode::decode_thread_message_reply;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::records::{MessageStatus, ThreadMessage, ThreadMessageData};
use http::header::{ACCEPT, AUTHORIZATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use url::Url;

use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_message_attachments::error::MessagesError;
use crate::azure_urls::url_parts::{encoded_query, first_query_values};
use crate::credentials::source::TokenReply;
use crate::endpoints::projection::TokenProvider;

const API_KEY: HeaderName = HeaderName::from_static("api-key");
const OPENAI_BETA: HeaderName = HeaderName::from_static("openai-beta");
// Python's quote(safe="!$&'()*+,;=:@") leaves these marks alone, and NON_ALPHANUMERIC encodes all of them.
const SAFE_PATH_MARKS: [(&str, &str); 17] = [
    ("%21", "!"),
    ("%24", "$"),
    ("%26", "&"),
    ("%27", "'"),
    ("%28", "("),
    ("%29", ")"),
    ("%2A", "*"),
    ("%2B", "+"),
    ("%2C", ","),
    ("%2D", "-"),
    ("%2E", "."),
    ("%3A", ":"),
    ("%3B", ";"),
    ("%3D", "="),
    ("%40", "@"),
    ("%5F", "_"),
    ("%7E", "~"),
];

// The SDK quotes a thread id as one path segment and appends /messages (openai/_utils/_path.py#L26).
fn thread_tail(thread_id: &str) -> Result<String, ThreadsError> {
    if thread_id.is_empty() {
        return Err(ThreadsError::EmptyThreadId);
    }
    if thread_id == "." || thread_id == ".." {
        return Err(ThreadsError::DotSegment {
            segment: thread_id.to_owned(),
        });
    }
    Ok(format!(
        "threads/{}/messages",
        quote_path_segment(thread_id)
    ))
}

pub(crate) async fn send<T, R>(
    client: &AzureAssistantsClient<'_, T>,
    method: Method,
    thread_id: &str,
    body: RequestBody,
) -> Result<R, MessagesError>
where
    T: AssistantsTransport,
    R: DeserializeOwned,
{
    let request = outbound(client, method, &thread_tail(thread_id)?, body)?;
    let reply = send_checked(client, request).await?;
    Ok(decode_thread_message_reply(&reply)?)
}

pub async fn append_message<T>(
    client: &AzureAssistantsClient<'_, T>,
    thread_id: &str,
    data: &ThreadMessageData,
) -> Result<ThreadMessage, MessagesError>
where
    T: AssistantsTransport,
{
    let body = serde_json::to_vec(data).map_err(|source| ThreadsError::Encode { source })?;
    let mut message: ThreadMessage =
        send(client, Method::POST, thread_id, RequestBody::Json(body)).await?;
    // LiteLLM fills an absent or null status with completed before returning the message (assistants.py#L221-L226).
    if message.status.is_none() {
        message.status = Some(MessageStatus::Completed);
    }
    Ok(message)
}

fn outbound<T>(
    client: &AzureAssistantsClient<'_, T>,
    method: Method,
    tail: &str,
    body: RequestBody,
) -> Result<OutboundRequest, MessagesError> {
    let joined = if client.base_url.ends_with('/') {
        format!("{}{tail}", client.base_url)
    } else {
        format!("{}/{tail}", client.base_url)
    };
    let mut url = Url::parse(&joined).map_err(|source| ThreadsError::BaseUrl { source })?;
    let query = merged_query(url.query(), &client.api_version);
    url.set_query(query.as_deref());
    let mut headers = auth_headers(client)?;
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(OPENAI_BETA, HeaderValue::from_static("assistants=v2"));
    Ok(OutboundRequest {
        method,
        url,
        query: Vec::new(),
        headers,
        body,
        deadline: client.deadline,
    })
}

// The SDK sends a static token alone, the provider token next to a real key, otherwise only the key.
fn auth_headers<T>(client: &AzureAssistantsClient<'_, T>) -> Result<HeaderMap, MessagesError> {
    let mut headers = HeaderMap::new();
    let mut bearer = client.token.clone();
    if bearer.is_none()
        && let Some(provider) = client
            .inputs
            .as_ref()
            .and_then(|inputs| inputs.token_provider.as_ref())
    {
        bearer = Some(provider_token(provider)?);
    }
    let api_key = client
        .key
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty());
    if let Some(token) = bearer.as_ref() {
        let mut header = HeaderValue::try_from(format!("Bearer {}", token.expose_secret()))
            .map_err(|source| ThreadsError::Header { source })?;
        header.set_sensitive(true);
        headers.insert(AUTHORIZATION, header);
    }
    if client.token.is_none()
        && let Some(key) = api_key
    {
        let mut header = HeaderValue::try_from(key.expose_secret())
            .map_err(|source| ThreadsError::Header { source })?;
        header.set_sensitive(true);
        headers.insert(API_KEY, header);
    }
    Ok(headers)
}

fn provider_token(provider: &TokenProvider<'_>) -> Result<SecretString, MessagesError> {
    match provider.0.token() {
        Ok(TokenReply::Text(token)) if !token.expose_secret().is_empty() => Ok(token),
        Ok(TokenReply::Text(_) | TokenReply::NonText { .. }) => {
            Err(MessagesError::from(DefinitionsError::ProviderReply))
        }
        Err(failure) => Err(MessagesError::from(DefinitionsError::Provider {
            class: failure.class,
            message: failure.message,
        })),
    }
}

async fn send_checked<T>(
    client: &AzureAssistantsClient<'_, T>,
    request: OutboundRequest,
) -> Result<Vec<u8>, MessagesError>
where
    T: AssistantsTransport,
{
    let response = client
        .transport
        .send(request)
        .await
        .map_err(|source| ThreadsError::Transport { source })?;
    if response.status >= 400 {
        return Err(MessagesError::from(ThreadsError::Status {
            status: response.status,
            body: response.body,
        }));
    }
    Ok(response.body)
}

// A relative path joins the base URL's raw path, so a base URL query swallows the tail exactly as the SDK's does.
fn merged_query(query: Option<&str>, api_version: &str) -> Option<String> {
    let mut pairs = first_query_values(query);
    match pairs.iter_mut().find(|(name, _)| name == "api-version") {
        Some((_, value)) => api_version.clone_into(value),
        None => pairs.push(("api-version".to_owned(), api_version.to_owned())),
    }
    // urlencode drops a pair whose value is empty, so an empty api-version leaves no query at all.
    pairs.retain(|(_, value)| !value.is_empty());
    encoded_query(&pairs)
}

fn quote_path_segment(segment: &str) -> String {
    let mut escaped = utf8_percent_encode(segment, NON_ALPHANUMERIC).to_string();
    for (encoded, mark) in SAFE_PATH_MARKS {
        escaped = escaped.replace(encoded, mark);
    }
    escaped
}
