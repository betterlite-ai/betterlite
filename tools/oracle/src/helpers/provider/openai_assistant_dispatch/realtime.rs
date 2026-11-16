use async_trait::async_trait;
use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_contracts::close::close_after_handshake_refusal;
use betterlite_core::api::realtime_contracts::error::WebSocketError;
use betterlite_core::api::realtime_contracts::transport::{
    ClientWebSocket, ScopedWebSocket, WebSocketScope,
};
use betterlite_openai::realtime::failure::ClientAction;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_assistant_dispatch::OpenAiAssistantPrepared;
use betterlite_provider::openai_assistant_dispatch::call::{
    OpenAiAssistantOperation, RealtimeCall, RealtimeFailureOwned,
};
use serde_json::{Value, json};

use super::input::{Input, SslVerification};
use super::invalid_input;

const FAILURE: &str = "stop";

// The proxy's client socket: a refused handshake ends in the close frame recorded here.
#[derive(Default)]
struct Client {
    scope: WebSocketScope,
    closed: Value,
}

impl ScopedWebSocket for Client {
    fn scope(&self) -> &WebSocketScope {
        &self.scope
    }
}

#[async_trait]
impl ClientWebSocket for Client {
    async fn send_text(&mut self, _data: &str) -> Result<(), WebSocketError> {
        Ok(())
    }

    async fn receive_text(&mut self) -> Result<String, WebSocketError> {
        Err(WebSocketError {
            message: String::from("the probe client sends no frame"),
        })
    }

    async fn close(&mut self, code: u16, reason: Option<&str>) -> Result<(), WebSocketError> {
        self.closed = json!({"code": code, "reason": reason});
        Ok(())
    }
}

pub(super) fn connect(input: &Input) -> OpenAiAssistantOperation {
    let ssl_verification = match input.ssl_verification {
        SslVerification::Enabled => Verification::Enabled,
        SslVerification::Disabled => Verification::Disabled,
    };
    OpenAiAssistantOperation::Realtime(RealtimeCall::Connect {
        api_base: input.api_base.clone(),
        api_key: input.api_key.clone(),
        model: input.model.clone(),
        query_params: input.query_params.clone(),
        beta_realtime: input.openai_beta,
        ssl_verification,
        max_message_size_bytes: input.max_message_size_bytes,
    })
}

// The Python probe's connection raises InvalidStatus for a handshake status and RuntimeError("stop") otherwise.
pub(super) fn probe_failure(input: &Input) -> RealtimeFailureOwned {
    match (input.handshake_status, input.close_error.clone()) {
        (Some(status_code), _) => RealtimeFailureOwned::Handshake { status_code },
        (None, None) => RealtimeFailureOwned::Other {
            message: String::from(FAILURE),
        },
        (None, Some(close_error)) => RealtimeFailureOwned::CloseFailed {
            message: String::from(FAILURE),
            close_error,
        },
    }
}

// Python reports the websockets.connect arguments its fake records, with the headers in name order.
pub(super) fn session_view(prepared: &OpenAiAssistantPrepared) -> Value {
    let headers: Vec<[&str; 2]> = prepared
        .headers
        .iter()
        .map(|(name, value)| [name.as_str(), value.as_str()])
        .collect();
    json!({
        "url": prepared.url,
        "headers": headers,
        "max_size": prepared.max_message_size_bytes,
        "ssl": prepared.ssl,
    })
}

// The close frame the client socket ends with, as LiteLLM's websocket.close receives it.
pub(super) async fn closed(action: &ClientAction) -> Result<Value, Box<HelperError>> {
    match action {
        ClientAction::Close(frame) | ClientAction::IgnoreCloseFailure { attempted: frame } => {
            Ok(json!({"code": frame.code, "reason": frame.reason}))
        }
        ClientAction::RefuseHandshake { status_code } => {
            let mut client = Client::default();
            close_after_handshake_refusal(&mut client, *status_code)
                .await
                .map_err(|error| invalid_input(error.to_string()))?;
            Ok(client.closed)
        }
    }
}
