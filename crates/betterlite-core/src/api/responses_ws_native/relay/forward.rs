use std::error::Error;
use std::string::FromUtf8Error;

use async_trait::async_trait;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket, ResponsesWebSocketMessage,
};
use futures_util::future::{Either, select};
use thiserror::Error;

use super::ResponsesWsRelay;
use super::backend::{BackendError, NativeWsBackend};
use super::capture::CaptureRelay as _;
use crate::api::responses_ws_frame_policy::quota::enforce_or_reject_frame;
use crate::api::responses_ws_frame_policy::request::mask_response_create;
use crate::api::responses_ws_frame_policy::response::{
    mask_response_completed, unmask_response_event,
};
use crate::api::responses_ws_frame_policy::{
    DELTA_EVENT_TYPES, FramePolicyError, OUTPUT_DONE_EVENT_TYPES,
};
use crate::api::responses_ws_native::error::WsNativeError;
use crate::api::responses_ws_native::request::json_object;
use crate::cost::realtime::error::RealtimeCostError;

#[derive(Debug, Error)]
pub(super) enum BackendEnd<ClientError, BackendSource, GuardError>
where
    GuardError: Error + 'static,
{
    #[error(transparent)]
    Event(#[from] WsNativeError),
    #[error("{0}")]
    Frame(#[from] FramePolicyError<GuardError>),
    #[error("{0}")]
    Client(#[source] ClientError),
    #[error("{0}")]
    Backend(#[source] BackendSource),
    #[error("{0}")]
    Utf8(#[from] FromUtf8Error),
    #[error("the backend does not accept the decode argument")]
    UnsupportedDecode,
}

#[derive(Debug, Error)]
pub(super) enum ClientRelayError<ClientError, BackendSource, GuardError>
where
    GuardError: Error + 'static,
{
    #[error("{0}")]
    Receive(#[source] ClientError),
    #[error("{0}")]
    Frame(#[from] FramePolicyError<GuardError>),
    #[error("{}", class_message(.0))]
    Rejected(ErrorClass),
    #[error(transparent)]
    Event(#[from] WsNativeError),
    #[error("{0}")]
    Backend(#[source] BackendSource),
}

// multiple_inherent_impl leaves relay.rs the type's one inherent impl, so the relay loop is a trait
#[async_trait]
pub trait ForwardRelay {
    async fn backend_to_client(&self) -> Result<(), RealtimeCostError>;

    async fn client_to_backend(&self);

    async fn bidirectional_forward(&self) -> Option<ErrorClass>;
}

// The frame steps behind the loop stay off the public trait because they name the relay's own errors
#[async_trait]
pub(super) trait ForwardSteps<ClientError, BackendSource, GuardError>
where
    ClientError: Error + 'static,
    BackendSource: Error + 'static,
    GuardError: Error + 'static,
{
    async fn next_backend_text(
        &self,
    ) -> Result<Option<String>, BackendEnd<ClientError, BackendSource, GuardError>>;

    async fn forward_client_frames(
        &self,
    ) -> Result<(), ClientRelayError<ClientError, BackendSource, GuardError>>;

    async fn accept_client_frame(
        &self,
        message: &str,
    ) -> Result<(), ClientRelayError<ClientError, BackendSource, GuardError>>;
}

#[async_trait]
impl<Guard, Client, Backend> ForwardRelay for ResponsesWsRelay<'_, Guard, Client, Backend>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Send + Sync,
    Guard::Error: 'static,
    Client: ResponsesClientWebSocket + Sync,
    Client::Error: 'static,
    Backend: NativeWsBackend,
    Backend::Error: 'static,
{
    async fn backend_to_client(&self) -> Result<(), RealtimeCostError> {
        let forwarded: Result<(), BackendEnd<Client::Error, Backend::Error, Guard::Error>> =
            async {
                while let Some(text) = self.next_backend_text().await? {
                    let policy = self.policy();
                    let wrapped = {
                        let request_data = self.request_data.lock().await;
                        let unmasked = unmask_response_event(&policy, &request_data, &text)?;
                        let masked =
                            mask_response_completed(&policy, &request_data, &unmasked).await?;
                        drop(request_data);
                        self.wrap_response_event(&masked)?
                    };
                    self.store_event(&wrapped).await?;
                    self.setup
                        .websocket
                        .send_text(&wrapped)
                        .await
                        .map_err(BackendEnd::Client)?;
                }
                Ok(())
            }
            .await;
        if let Err(end) = forwarded {
            tracing::error!("Error in responses WS backend_to_client: {end}");
        }
        self.log_messages().await
    }

    async fn client_to_backend(&self) {
        if let Err(error) = self.forward_client_frames().await {
            tracing::debug!("Responses WS client_to_backend ended: {error}");
        }
    }

    async fn bidirectional_forward(&self) -> Option<ErrorClass> {
        let mut backend_forward = Box::pin(self.backend_to_client());
        let mut client_forward = Box::pin(self.client_to_backend());
        match select(client_forward.as_mut(), backend_forward.as_mut()).await {
            Either::Left(((), _backend_pending)) => {
                if let Err(error) = self.log_messages().await {
                    tracing::error!("Error in responses WS backend_to_client: {error}");
                }
            }
            Either::Right((_logged, client_pending)) => {
                client_pending.await;
            }
        }
        let _close = self.setup.backend.close().await;
        self.failure_exception().await
    }
}

#[async_trait]
impl<Guard, Client, Backend> ForwardSteps<Client::Error, Backend::Error, Guard::Error>
    for ResponsesWsRelay<'_, Guard, Client, Backend>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Send + Sync,
    Guard::Error: 'static,
    Client: ResponsesClientWebSocket + Sync,
    Client::Error: 'static,
    Backend: NativeWsBackend,
    Backend::Error: 'static,
{
    async fn next_backend_text(
        &self,
    ) -> Result<Option<String>, BackendEnd<Client::Error, Backend::Error, Guard::Error>> {
        loop {
            let outcome = self.setup.backend.recv_raw().await;
            let received = if matches!(&outcome, Err(BackendError::UnsupportedDecode)) {
                self.setup.backend.recv().await
            } else {
                outcome
            };
            let message = match received {
                Ok(message) => message,
                Err(BackendError::Closed(source)) => {
                    tracing::debug!("Responses WS backend connection closed: {source}");
                    return Ok(None);
                }
                Err(BackendError::Failed(source)) => return Err(BackendEnd::Backend(source)),
                Err(BackendError::UnsupportedDecode) => {
                    return Err(BackendEnd::UnsupportedDecode);
                }
            };
            let text = match message {
                ResponsesWebSocketMessage::Text(text) => text,
                ResponsesWebSocketMessage::Bytes(bytes) => String::from_utf8(bytes)?,
            };
            let guarded = !self
                .setup
                .output_guardrail_callbacks
                .unwrap_or_default()
                .is_empty();
            if !guarded || !output_dropped(&text)? {
                return Ok(Some(text));
            }
        }
    }

    async fn forward_client_frames(
        &self,
    ) -> Result<(), ClientRelayError<Client::Error, Backend::Error, Guard::Error>> {
        if let Some(first) = self.setup.first_message.clone() {
            self.accept_client_frame(&first).await?;
        }
        loop {
            let message = self
                .setup
                .websocket
                .receive_text()
                .await
                .map_err(ClientRelayError::Receive)?;
            self.accept_client_frame(&message).await?;
        }
    }

    async fn accept_client_frame(
        &self,
        message: &str,
    ) -> Result<(), ClientRelayError<Client::Error, Backend::Error, Guard::Error>> {
        let accepted = enforce_or_reject_frame(&self.policy(), self.setup.websocket, message)
            .await
            .map_err(ClientRelayError::Rejected)?;
        if !accepted {
            return Ok(());
        }
        let mut request_data = self.request_data.lock().await;
        let masked = mask_response_create(&self.policy(), &mut request_data, message).await?;
        drop(request_data);
        self.store_input(&masked).await;
        self.store_event(&masked).await?;
        self.setup
            .backend
            .send(&masked)
            .await
            .map_err(ClientRelayError::Backend)
    }
}

fn output_dropped(message: &str) -> Result<bool, WsNativeError> {
    let Some(fields) = json_object(message)? else {
        return Ok(false);
    };
    let Some(Json::Text(kind)) = fields.get("type") else {
        return Ok(false);
    };
    Ok(DELTA_EVENT_TYPES.contains(&kind.as_str())
        || OUTPUT_DONE_EVENT_TYPES.contains(&kind.as_str()))
}
