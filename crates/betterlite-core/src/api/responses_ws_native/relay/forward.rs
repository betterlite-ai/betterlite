use std::error::Error;
use std::fmt::Display;

use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket, ResponsesWebSocketMessage,
};
use futures_util::future::{Either, select};
use thiserror::Error;

use super::ResponsesWsRelay;
use super::backend::{BackendError, NativeWsBackend};
use crate::api::responses_ws_frame_policy::quota::enforce_or_reject_frame;
use crate::api::responses_ws_frame_policy::request::mask_response_create;
use crate::api::responses_ws_frame_policy::response::{
    mask_response_completed, unmask_response_event,
};
use crate::api::responses_ws_frame_policy::{DELTA_EVENT_TYPES, OUTPUT_DONE_EVENT_TYPES};
use crate::api::responses_ws_native::error::WsNativeError;
use crate::api::responses_ws_native::request::json_object;
use crate::cost::realtime::error::RealtimeCostError;

struct BackendEnd {
    error: String,
}

impl BackendEnd {
    fn failed<Error>(error: Error) -> Self
    where
        Error: Display,
    {
        Self {
            error: error.to_string(),
        }
    }
}

#[derive(Debug, Error)]
enum ClientRelayError {
    #[error("{0}")]
    Frame(String),
    #[error(transparent)]
    Event(#[from] WsNativeError),
    #[error("{0}")]
    Send(String),
}

impl<'call, Guard, Client, Backend> ResponsesWsRelay<'call, Guard, Client, Backend>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Send + Sync,
    Guard::Error: 'static,
    Client: ResponsesClientWebSocket + Sync,
    Backend: NativeWsBackend,
{
    pub async fn backend_to_client(&self) -> Result<(), RealtimeCostError> {
        if let Err(end) = self.forward_backend_events().await {
            tracing::error!("Error in responses WS backend_to_client: {}", end.error);
        }
        self.log_messages().await
    }

    pub async fn client_to_backend(&self) {
        if let Err(error) = self.forward_client_frames().await {
            tracing::debug!("Responses WS client_to_backend ended: {error}");
        }
    }

    pub async fn bidirectional_forward(&self) -> Option<ErrorClass> {
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

    async fn forward_backend_events(&self) -> Result<(), BackendEnd> {
        while let Some(text) = self.next_backend_text().await? {
            let wrapped = self.transform_backend_frame(&text).await?;
            self.store_event(&wrapped)
                .await
                .map_err(BackendEnd::failed)?;
            self.setup
                .websocket
                .send_text(&wrapped)
                .await
                .map_err(BackendEnd::failed)?;
        }
        Ok(())
    }

    async fn receive_message(
        &self,
    ) -> Result<ResponsesWebSocketMessage, BackendError<Backend::Error>> {
        let outcome = self.setup.backend.recv_raw().await;
        if matches!(&outcome, Err(BackendError::UnsupportedDecode)) {
            return self.setup.backend.recv().await;
        }
        outcome
    }

    async fn next_backend_text(&self) -> Result<Option<String>, BackendEnd> {
        loop {
            let message = match self.receive_message().await {
                Ok(message) => message,
                Err(BackendError::Closed { source }) => {
                    tracing::debug!("Responses WS backend connection closed: {source}");
                    return Ok(None);
                }
                Err(error) => return Err(backend_end(error)),
            };
            let text = match message {
                ResponsesWebSocketMessage::Text(text) => text,
                ResponsesWebSocketMessage::Bytes(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => text,
                    Err(error) => return Err(BackendEnd::failed(error)),
                },
            };
            let guarded = !self
                .setup
                .output_guardrail_callbacks
                .unwrap_or_default()
                .is_empty();
            if !guarded || !output_dropped(&text).map_err(BackendEnd::failed)? {
                return Ok(Some(text));
            }
        }
    }

    async fn transform_backend_frame(&self, message: &str) -> Result<String, BackendEnd> {
        let policy = self.policy();
        let request_data = self.request_data.lock().await;
        let unmasked =
            unmask_response_event(&policy, &request_data, message).map_err(BackendEnd::failed)?;
        let masked = mask_response_completed(&policy, &request_data, &unmasked)
            .await
            .map_err(BackendEnd::failed)?;
        self.wrap_response_event(&masked)
            .map_err(BackendEnd::failed)
    }

    async fn forward_client_frames(&self) -> Result<(), ClientRelayError> {
        let policy = self.policy();
        if let Some(first) = self.setup.first_message.as_deref()
            && enforce_or_reject_frame(&policy, self.setup.websocket, first)
                .await
                .map_err(|error| ClientRelayError::Frame(class_message(&error)))?
        {
            self.relay_client_frame(first).await?;
        }
        loop {
            let message = self
                .setup
                .websocket
                .receive_text()
                .await
                .map_err(|error| ClientRelayError::Frame(error.to_string()))?;
            let accepted = enforce_or_reject_frame(&policy, self.setup.websocket, &message)
                .await
                .map_err(|error| ClientRelayError::Frame(class_message(&error)))?;
            if accepted {
                self.relay_client_frame(&message).await?;
            }
        }
    }

    async fn relay_client_frame(&self, message: &str) -> Result<(), ClientRelayError> {
        let masked = self.masked_client_frame(message).await?;
        self.store_input(&masked).await;
        self.store_event(&masked).await?;
        self.setup
            .backend
            .send(&masked)
            .await
            .map_err(|error| ClientRelayError::Send(error.to_string()))
    }

    async fn masked_client_frame(&self, message: &str) -> Result<String, ClientRelayError> {
        let policy = self.policy();
        let mut request_data = self.request_data.lock().await;
        mask_response_create(&policy, &mut request_data, message)
            .await
            .map_err(|error| ClientRelayError::Frame(error.to_string()))
            .map(|masked| masked.into_owned())
    }
}

fn backend_end<Source>(error: BackendError<Source>) -> BackendEnd
where
    Source: Error + 'static,
{
    match error {
        BackendError::UnsupportedDecode => {
            BackendEnd::failed("the backend does not accept the decode argument")
        }
        BackendError::Closed { source } | BackendError::Failed { source } => {
            BackendEnd::failed(source)
        }
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
