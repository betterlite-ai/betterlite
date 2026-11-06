use std::sync::PoisonError;

use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, iter};

use crate::client::A2AClient;
use crate::client::convert::{to_compat_stream_response, to_core_send_message_request};
use crate::client::error::ClientError;
use crate::client::records::compat::SendMessageRequest;
use crate::client::records::response::{SendMessageResponse, StreamResult};
use crate::client::transport::EventStream;

async fn open(
    client: &A2AClient,
    request: &SendMessageRequest,
) -> Result<EventStream, ClientError> {
    let core = to_core_send_message_request(request.clone())?;
    let request_id = {
        let mut ids = client
            .request_ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        ids.next_request_id()
    };
    client.send_message(&core, &request_id).await
}

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
        &response.result,
        StreamResult::Message(_) | StreamResult::Task(_)
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
    let events = match open(client, request).await {
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
