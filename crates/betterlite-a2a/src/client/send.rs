use betterlite_core::agents::json::AgentJson;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, iter};
use uuid::Uuid;

use crate::client::A2AClient;
use crate::client::convert::{to_compat_stream_response, to_core_send_message_request};
use crate::client::error::ClientError;
use crate::client::records::SendMessageResponse;
use crate::client::records::compat::SendMessageRequest;
pub async fn send_message(
    client: &A2AClient,
    request: &SendMessageRequest,
) -> Result<SendMessageResponse, ClientError> {
    let mut events = stream_messages(client, request).await;
    let mut last = None;
    while let Some(item) = events.next().await {
        last = Some(item?);
    }
    let Some(response) = last else {
        return Err(ClientError::Final {
            message: "A2A send_message failed: no response received from agent.",
        });
    };
    if matches!(
        response.result.get("kind").and_then(AgentJson::as_str),
        Some("message" | "task")
    ) {
        return Ok(response);
    }
    Err(ClientError::Final {
        message: "A2A send_message failed: the agent's final event is not a Message or Task result.",
    })
}
#[must_use]
pub async fn stream_messages(
    client: &A2AClient,
    request: &SendMessageRequest,
) -> BoxStream<'static, Result<SendMessageResponse, ClientError>> {
    let core = match to_core_send_message_request(request.clone()) {
        Ok(core) => core,
        Err(error) => return iter(Some(Err(ClientError::Convert(error)))).boxed(),
    };
    let events = match client
        .send_message(&core, &Uuid::new_v4().to_string())
        .await
    {
        Ok(events) => events,
        Err(error) => return iter(Some(Err(error))).boxed(),
    };
    let request_id = request.id.clone();
    async_stream::stream! {
        let mut pending = events;
        while let Some(event) = pending.next().await {
            match event {
                Ok(chunk) => match to_compat_stream_response(chunk, request_id.clone()) {
                    Ok(response) => yield Ok(response),
                    Err(error) => {
                        yield Err(ClientError::Convert(error));
                        return;
                    }
                },
                Err(error) => {
                    yield Err(error);
                    return;
                }
            }
        }
    }
    .boxed()
}
