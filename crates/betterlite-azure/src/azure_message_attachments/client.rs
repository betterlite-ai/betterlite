use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::decode::decode_thread_message_reply;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::records::{MessageStatus, ThreadMessage, ThreadMessageData};
use http::Method;
use serde::de::DeserializeOwned;

use crate::azure_assistant_definitions::client::{AzureAssistantsClient, quote_path_segment};
use crate::azure_message_attachments::error::MessagesError;

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
    let request = client.outbound(method, &thread_tail(thread_id)?, body)?;
    let reply = client.send_checked(request).await?;
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
