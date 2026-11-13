use std::sync::Arc;

use betterlite_core::a2a::exception_mapping::{
    A2AException, RecoveryError, handle_a2a_localhost_retry, map_a2a_exception,
};
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use thiserror::Error;

use crate::agent_card::AgentCard;
use crate::card_resolver::card_url::get_agent_card_url;
use crate::client::error::{ClientError, exception_to_a2a};
use crate::client::records::compat::SendMessageRequest;
use crate::client::records::response::SendMessageResponse;
use crate::client::send::{send_message, stream_messages};
use crate::client::{A2AClient, NativeA2aSdk};
use crate::errors::A2AError;

#[derive(Debug, Error)]
pub enum SendError {
    #[error(transparent)]
    A2A(Box<A2AError>),
    #[error(transparent)]
    Client(Box<ClientError>),
    #[error(transparent)]
    Recovery(Box<RecoveryError<ClientError>>),
    #[error("A2A send_message failed: no response received after retry attempts.")]
    NoResponse,
}

pub struct SendTarget {
    pub agent_card: Option<AgentCard>,
    pub card_url: Option<String>,
    pub api_base: Option<String>,
    pub agent_name: Option<String>,
}

async fn recover(
    error: ClientError,
    client: &A2AClient,
    target: &mut SendTarget,
    streaming: bool,
) -> Result<A2AClient, SendError> {
    let mapped = map_a2a_exception(
        &NativeA2aSdk,
        &error,
        target.card_url.as_deref(),
        target.api_base.as_deref(),
        target.agent_name.as_deref(),
    );
    let A2AException::LocalhostUrl(localhost) = mapped else {
        return Err(SendError::A2A(Box::new(exception_to_a2a(mapped))));
    };
    let mut rebuilt = handle_a2a_localhost_retry(
        &NativeA2aSdk,
        &localhost,
        target.agent_card.as_mut(),
        client,
        streaming,
    )
    .await
    .map_err(|source| SendError::Recovery(Box::new(source)))?;
    rebuilt.request_ids = Arc::clone(&client.request_ids);
    target.card_url = target
        .agent_card
        .as_ref()
        .and_then(get_agent_card_url)
        .map(str::to_owned);
    Ok(rebuilt)
}

pub async fn execute_send_with_retry(
    mut client: A2AClient,
    request: &SendMessageRequest,
    target: &mut SendTarget,
) -> Result<SendMessageResponse, SendError> {
    for _attempt in 0..2_u8 {
        match send_message(&client, request).await {
            Ok(response) => return Ok(response),
            Err(error) => client = recover(error, &client, target, false).await?,
        }
    }
    Err(SendError::NoResponse)
}

#[must_use]
pub fn execute_stream_with_retry(
    client: A2AClient,
    request: SendMessageRequest,
    target: SendTarget,
) -> BoxStream<'static, Result<SendMessageResponse, SendError>> {
    async_stream::stream! {
        let mut running_client = client;
        let mut running_target = target;
        let mut response_started = false;
        let mut stream_succeeded = false;
        'attempts: for _attempt in 0..2_u8 {
            let mut chunks = stream_messages(&running_client, &request).await;
            while let Some(item) = chunks.next().await {
                match item {
                    Ok(chunk) => {
                        response_started = true;
                        yield Ok(chunk);
                    }
                    Err(error) => {
                        if response_started {
                            yield Err(SendError::Client(Box::new(error)));
                            return;
                        }
                        match recover(error, &running_client, &mut running_target, true).await {
                            Ok(rebuilt) => {
                                running_client = rebuilt;
                                continue 'attempts;
                            }
                            Err(failure) => {
                                yield Err(failure);
                                return;
                            }
                        }
                    }
                }
            }
            stream_succeeded = true;
            break;
        }
        if !stream_succeeded {
            yield Err(SendError::NoResponse);
        }
    }
    .boxed()
}
