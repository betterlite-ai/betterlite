use std::time::Duration;

use betterlite_config::sections::openai_credentials::DEFAULT_API_BASE;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::{ACCEPT, AUTHORIZATION};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use crate::assistants::client::SdkClient;
use crate::assistants::transport::AssistantsTransport;
use crate::threads::decode::decode_thread_message_reply;
use crate::threads::error::ThreadsError;
use crate::threads::operations::{ThreadCall, ThreadReply};
use crate::threads::records::{MessageStatus, ThreadMessage};

const OPENAI_BETA: HeaderName = HeaderName::from_static("openai-beta");
const OPENAI_ORGANIZATION: HeaderName = HeaderName::from_static("openai-organization");
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
// The OpenAI SDK quotes a path id with Python's quote(safe="!$&'()*+,;=:@"), so "/" is encoded and the unreserved marks stay.
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

fn endpoint(base: Option<&str>, tail: &str) -> Result<Url, ThreadsError> {
    let mut url = Url::parse(base.unwrap_or(DEFAULT_API_BASE))
        .map_err(|source| ThreadsError::BaseUrl { source })?;
    let mut path = url.path().to_owned();
    if !path.ends_with('/') {
        path.push('/');
    }
    path.push_str(tail);
    url.set_path(&path);
    Ok(url)
}

fn bearer(api_key: &SecretString) -> Result<HeaderValue, ThreadsError> {
    let mut value = HeaderValue::try_from(format!("Bearer {}", api_key.expose_secret()))
        .map_err(|source| ThreadsError::Header { source })?;
    value.set_sensitive(true);
    Ok(value)
}

fn headers(api_key: &SecretString, organization: Option<&str>) -> Result<HeaderMap, ThreadsError> {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, bearer(api_key)?);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(OPENAI_BETA, HeaderValue::from_static("assistants=v2"));
    if let Some(name) = organization {
        let value =
            HeaderValue::try_from(name).map_err(|source| ThreadsError::Header { source })?;
        headers.insert(OPENAI_ORGANIZATION, value);
    }
    Ok(headers)
}

fn thread_tail(thread_id: &str, suffix: &str) -> Result<String, ThreadsError> {
    if thread_id.is_empty() {
        return Err(ThreadsError::EmptyThreadId);
    }
    if thread_id == "." || thread_id == ".." {
        return Err(ThreadsError::DotSegment {
            segment: thread_id.to_owned(),
        });
    }
    Ok(format!(
        "threads/{}{suffix}",
        utf8_percent_encode(thread_id, PATH_SEGMENT)
    ))
}

async fn call<T, R>(
    client: &SdkClient<T>,
    method: Method,
    tail: &str,
    body: RequestBody,
) -> Result<R, ThreadsError>
where
    T: AssistantsTransport,
    R: DeserializeOwned,
{
    let api_key = client.api_key.as_ref().ok_or(ThreadsError::MissingApiKey)?;
    let request = OutboundRequest {
        method,
        url: endpoint(client.base_url.as_deref(), tail)?,
        query: Vec::new(),
        headers: headers(api_key, client.organization.as_deref())?,
        body,
        deadline: client.timeout.unwrap_or(DEFAULT_TIMEOUT),
    };
    let response = client
        .transport
        .send(request)
        .await
        .map_err(|source| ThreadsError::Transport { source })?;
    if response.status >= 400 {
        return Err(ThreadsError::Status {
            status: response.status,
            body: response.body,
        });
    }
    decode_thread_message_reply(&response.body)
}

fn json_body<T>(data: &T) -> Result<RequestBody, ThreadsError>
where
    T: Serialize,
{
    serde_json::to_vec(data)
        .map(RequestBody::Json)
        .map_err(|source| ThreadsError::Encode { source })
}

// LiteLLM fills a status the SDK returned as absent or null with completed before validating the message.
const fn with_completed_status(mut message: ThreadMessage) -> ThreadMessage {
    if message.status.is_none() {
        message.status = Some(MessageStatus::Completed);
    }
    message
}

pub async fn send<T>(
    client: &SdkClient<T>,
    request: ThreadCall<'_>,
) -> Result<ThreadReply, ThreadsError>
where
    T: AssistantsTransport,
{
    match request {
        ThreadCall::Create(data) => {
            let thread = call(client, Method::POST, "threads", json_body(data)?).await?;
            Ok(ThreadReply::Thread(thread))
        }
        ThreadCall::Retrieve(thread_id) => {
            let thread = call(
                client,
                Method::GET,
                &thread_tail(thread_id, "")?,
                RequestBody::Empty,
            )
            .await?;
            Ok(ThreadReply::Thread(thread))
        }
        ThreadCall::AddMessage { thread_id, data } => {
            let message: ThreadMessage = call(
                client,
                Method::POST,
                &thread_tail(thread_id, "/messages")?,
                json_body(data)?,
            )
            .await?;
            Ok(ThreadReply::Message(with_completed_status(message)))
        }
        ThreadCall::ListMessages(thread_id) => {
            let page = call(
                client,
                Method::GET,
                &thread_tail(thread_id, "/messages")?,
                RequestBody::Empty,
            )
            .await?;
            Ok(ThreadReply::Page(page))
        }
    }
}
