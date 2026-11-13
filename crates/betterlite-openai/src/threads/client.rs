use betterlite_http::proxy::outbound::RequestBody;
use http::Method;
use percent_encoding::utf8_percent_encode;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::assistants::client::{PATH_SEGMENT, SdkClient};
use crate::assistants::transport::AssistantsTransport;
use crate::threads::decode::decode_thread_message_reply;
use crate::threads::error::ThreadsError;
use crate::threads::operations::{ThreadCall, ThreadReply};
use crate::threads::records::{MessageStatus, ThreadMessage};

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
    let request = client
        .outbound(method, tail, Vec::new(), body)
        .map_err(ThreadsError::from)?;
    let reply = client
        .send_checked(request)
        .await
        .map_err(ThreadsError::from)?;
    decode_thread_message_reply(&reply)
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
            Ok(ThreadReply::Message(Box::new(with_completed_status(
                message,
            ))))
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
