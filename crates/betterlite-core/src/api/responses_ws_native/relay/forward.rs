use std::error::Error;
use std::fmt::Display;

use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket, ResponsesWebSocketMessage,
};
use futures_util::future::{Either, select};

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

enum BackendEnd {
    Closed { error: String },
    Failed { error: String },
}

impl BackendEnd {
    fn failed(error: impl Display) -> Self {
        Self::Failed {
            error: error.to_string(),
        }
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
        BackendError::Closed { source } => BackendEnd::Closed {
            error: source.to_string(),
        },
        BackendError::Failed { source } => BackendEnd::failed(source),
    }
}

impl<'call, Guard, Client, Backend> ResponsesWsRelay<'call, Guard, Client, Backend>
where
    Guard: PresidioGuardrailCallback,
    Client: ResponsesClientWebSocket + Sync,
    Backend: NativeWsBackend,
{
    // the connection ends after final logging, whatever stopped the backend forward loop
    pub async fn backend_to_client(&self) -> Result<(), RealtimeCostError>
    where
        Guard: Sync,
        Guard::Settings: Send + Sync,
        Guard::Error: 'static,
    {
        match self.forward_backend_events().await {
            Ok(()) => {}
            Err(BackendEnd::Closed { error }) => {
                tracing::debug!("Responses WS backend connection closed: {error}");
            }
            Err(BackendEnd::Failed { error }) => {
                tracing::error!("Error in responses WS backend_to_client: {error}");
            }
        }
        self.log_messages().await
    }

    // the client loop swallows every failure after the debug line, keeping the relay alive until it ends
    pub async fn client_to_backend(&self)
    where
        Guard: Sync,
        Guard::Settings: Send + Sync,
        Guard::Error: 'static,
    {
        if let Err(error) = self.forward_client_frames().await {
            tracing::debug!("Responses WS client_to_backend ended: {error}");
        }
    }

    // whichever direction ends first decides the close path; the returned failure is the stored one
    pub async fn bidirectional_forward(&self) -> Option<ErrorClass>
    where
        Guard: Sync,
        Guard::Settings: Send + Sync,
        Guard::Error: 'static,
    {
        let mut backend_forward = Box::pin(self.backend_to_client());
        let mut client_forward = Box::pin(self.client_to_backend());
        match select(client_forward.as_mut(), backend_forward.as_mut()).await {
            Either::Left(((), _backend_pending)) => {
                if let Err(error) = self.log_messages().await {
                    tracing::error!("Error in responses WS backend_to_client: {error}");
                }
            }
            Either::Right(((), client_pending)) => {
                client_pending.await;
            }
        }
        let _close = self.setup.backend.close().await;
        self.failure_exception().await
    }

    async fn forward_backend_events(&self) -> Result<(), BackendEnd>
    where
        Guard: Sync,
        Guard::Settings: Send + Sync,
        Guard::Error: 'static,
    {
        loop {
            let message = self.receive_message().await.map_err(backend_end)?;
            let text = match message {
                ResponsesWebSocketMessage::Text(text) => text,
                ResponsesWebSocketMessage::Bytes(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => text,
                    Err(error) => return Err(BackendEnd::failed(error)),
                },
            };
            if !self
                .setup
                .output_guardrail_callbacks
                .unwrap_or_default()
                .is_empty()
            {
                match output_dropped(&text) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => return Err(BackendEnd::failed(error)),
                }
            }
            self.relay_backend_frame(&text).await?;
        }
    }

    // recv(decode=False) first; only a peer without that argument falls back to recv()
    async fn receive_message(
        &self,
    ) -> Result<ResponsesWebSocketMessage, BackendError<Backend::Error>> {
        let outcome = self.setup.backend.recv_raw().await;
        if matches!(&outcome, Err(BackendError::UnsupportedDecode)) {
            return self.setup.backend.recv().await;
        }
        outcome
    }

    async fn relay_backend_frame(&self, message: &str) -> Result<(), BackendEnd>
    where
        Guard: Sync,
        Guard::Settings: Send + Sync,
        Guard::Error: 'static,
    {
        let policy = self.policy();
        let (unmasked, masked) = {
            let request_data = self.request_data.lock().await;
            let unmasked = unmask_response_event(&policy, &request_data, message)
                .map_err(BackendEnd::failed)?;
            let masked = mask_response_completed(&policy, &request_data, &unmasked)
                .await
                .map_err(BackendEnd::failed)?;
            (unmasked.into_owned(), masked.into_owned())
        };
        let wrapped = self
            .wrap_response_event(&masked)
            .map_err(BackendEnd::failed)?;
        self.store_event(&wrapped)
            .await
            .map_err(BackendEnd::failed)?;
        self.setup
            .websocket
            .send_text(&wrapped)
            .await
            .map_err(BackendEnd::failed)
    }

    async fn forward_client_frames(&self) -> Result<(), String>
    where
        Guard: Sync,
        Guard::Settings: Send + Sync,
        Guard::Error: 'static,
    {
        let policy = self.policy();
        if let Some(first) = self.setup.first_message.as_deref()
            && enforce_or_reject_frame(&policy, self.setup.websocket, first)
                .await
                .map_err(|error| class_message(&error))?
        {
            self.relay_client_frame(first).await?;
        }
        loop {
            let message = self
                .setup
                .websocket
                .receive_text()
                .await
                .map_err(|error| error.to_string())?;
            let accepted = enforce_or_reject_frame(&policy, self.setup.websocket, &message)
                .await
                .map_err(|error| class_message(&error))?;
            if accepted {
                self.relay_client_frame(&message).await?;
            }
        }
    }

    // masking runs before storage, so every stored and logged client frame is the masked one
    async fn relay_client_frame(&self, message: &str) -> Result<(), String>
    where
        Guard: Sync,
        Guard::Settings: Send + Sync,
        Guard::Error: 'static,
    {
        let masked = {
            let policy = self.policy();
            let mut request_data = self.request_data.lock().await;
            mask_response_create(&policy, &mut request_data, message)
                .await
                .map_err(|error| error.to_string())?
                .into_owned()
        };
        self.store_input(&masked).await;
        self.store_event(&masked)
            .await
            .map_err(|error| error.to_string())?;
        self.setup
            .backend
            .send(&masked)
            .await
            .map_err(|error| error.to_string())
    }
}

// a falsey drop list still reads the event type first, so a non-object frame fails like LiteLLM's .get
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
