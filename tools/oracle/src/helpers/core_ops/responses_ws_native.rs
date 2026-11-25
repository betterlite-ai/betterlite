use std::collections::VecDeque;
use std::future::{Future, ready};
use std::io::{Error as IoError, ErrorKind};
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::responses_ws_frame_policy::RequestData;
use betterlite_core::api::responses_ws_native::relay::backend::{BackendError, NativeWsBackend};
use betterlite_core::api::responses_ws_native::relay::forward::ForwardRelay as _;
use betterlite_core::api::responses_ws_native::relay::{RelaySetup, ResponsesWsRelay};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket, ResponsesWebSocketMessage,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;
use tokio::task::yield_now;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Scenario;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        runtime.block_on(run_scenario(input))
    }
}

#[derive(Deserialize)]
pub(crate) struct Scenario {
    first_message: Option<String>,
    client_frames: Vec<String>,
    backend_frames: Vec<String>,
    request_data: IndexMap<String, Json>,
}

struct FakeSocket {
    sent: Mutex<Vec<String>>,
    frames: Mutex<VecDeque<String>>,
}

impl FakeSocket {
    fn new(frames: &[String]) -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            frames: Mutex::new(frames.iter().cloned().collect()),
        }
    }

    fn pop_frame(&self) -> Result<String, IoError> {
        self.frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| IoError::new(ErrorKind::BrokenPipe, "the socket closed"))
    }

    fn record(&self, text: &str) {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text.to_owned());
    }
}

// the pinned service awaits sleep(0) in every socket method, so the fake yields once per call too
impl ResponsesClientWebSocket for FakeSocket {
    type Error = IoError;

    async fn send_text(&self, data: &str) -> Result<(), Self::Error> {
        yield_now().await;
        self.record(data);
        Ok(())
    }

    async fn receive_text(&self) -> Result<String, Self::Error> {
        yield_now().await;
        self.pop_frame()
    }
}

impl NativeWsBackend for FakeSocket {
    type Error = IoError;

    async fn recv_raw(&self) -> Result<ResponsesWebSocketMessage, BackendError<Self::Error>> {
        yield_now().await;
        let frame = self.pop_frame().map_err(BackendError::<IoError>::Closed)?;
        Ok(ResponsesWebSocketMessage::Text(frame))
    }

    fn recv(
        &self,
    ) -> impl Future<Output = Result<ResponsesWebSocketMessage, BackendError<Self::Error>>> + Send + '_
    {
        self.recv_raw()
    }

    fn send<'call>(
        &'call self,
        message: &'call str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'call {
        self.send_text(message)
    }

    async fn close(&self) -> Result<(), Self::Error> {
        yield_now().await;
        Ok(())
    }
}

impl PresidioGuardrailCallback for FakeSocket {
    type Error = IoError;
    type Settings = ();

    fn presidio_settings(&self, _data: &RequestData) -> Option<()> {
        None
    }

    fn check_pii<'call>(
        &'call self,
        text: &'call str,
        _output_parse_pii: bool,
        _presidio_config: Option<&'call ()>,
        _request_data: &'call RequestData,
    ) -> impl Future<Output = Result<String, IoError>> + Send + 'call {
        ready(Ok(text.to_owned()))
    }
}

async fn run_scenario(scenario: &Scenario) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let client = FakeSocket::new(&scenario.client_frames);
    let backend = FakeSocket::new(&scenario.backend_frames);
    let relay: ResponsesWsRelay<'_, FakeSocket, FakeSocket, FakeSocket> =
        ResponsesWsRelay::new(RelaySetup {
            websocket: &client,
            backend: &backend,
            logging: None,
            user_api_key_dict: None,
            request_data: Some(scenario.request_data.clone()),
            first_message: scenario.first_message.clone(),
            guardrail_callbacks: None,
            output_guardrail_callbacks: None,
            quota_callbacks: None,
            authorized_model: None,
            custom_llm_provider: None,
            request_defaults: None,
            redactor: &redactor,
        });
    let failure_returned = relay.bidirectional_forward().await.is_some();
    let messages = relay.messages.lock().await.clone();
    let input_messages = relay.input_messages.lock().await.clone();
    Ok(json!({
        "sent_to_client": client.sent.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        "sent_to_backend": backend.sent.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        "messages": messages,
        "input_messages": input_messages,
        "litellm_metadata": relay.litellm_metadata,
        "failure_returned": failure_returned,
    }))
}
