use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::realtime_events::ack::{AckError, SessionAck};
use betterlite_bedrock::realtime_events::client::{
    ClientError, ClientMessages, RealtimeClientWebSocket,
};
use betterlite_bedrock::realtime_events::failure::{
    ProviderFailure, ProviderStreamError, raise_provider_failure,
};
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_bedrock::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    StreamError,
};
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use super::{AckStep, MODEL, StepOutput, failure};
use betterlite_oracle_helper_api::error::HelperError;

// The receive frames, the ack payloads and the Bedrock stream of one case stand in for two SDK clients
struct Transport {
    frames: Mutex<VecDeque<Option<String>>>,
    receive_calls: AtomicU32,
    sent: Mutex<Vec<String>>,
    send_error: Option<String>,
    stream_error: Option<String>,
}

#[async_trait]
impl RealtimeClientWebSocket for Transport {
    async fn receive_text(&self) -> Result<String, ClientError> {
        self.receive_calls.fetch_add(1, Ordering::SeqCst);
        let frame = self
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match frame {
            Some(Some(text)) => Ok(text),
            Some(None) | None => Err(ClientError {
                message: "the client sent no more frames".to_owned(),
                source: None,
            }),
        }
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data);
        match &self.send_error {
            Some(message) => Err(ClientError {
                message: message.clone(),
                source: None,
            }),
            None => Ok(()),
        }
    }

    async fn close(&self, _code: u16, _reason: Option<String>) {}
}

#[async_trait]
impl BedrockInputStream for Transport {
    async fn send(&self, _event: Json) -> Result<(), StreamError> {
        Ok(())
    }

    async fn close(&self) -> Result<(), StreamError> {
        Ok(())
    }
}

struct EmptyOutput;

#[async_trait]
impl BedrockOutputStream for EmptyOutput {
    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError> {
        Ok(None)
    }
}

#[async_trait]
impl BedrockBidirectionalStream for Transport {
    fn input_stream(&self) -> &dyn BedrockInputStream {
        self
    }

    async fn await_output(&self) -> Result<(Json, Box<dyn BedrockOutputStream>), StreamError> {
        match &self.stream_error {
            Some(message) => Err(StreamError {
                message: message.clone(),
                source: None,
            }),
            None => Ok((Json::Null, Box::new(EmptyOutput))),
        }
    }
}

fn current_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)
}

pub(super) fn client_messages(
    initial: Option<String>,
    received: Vec<Option<String>>,
    redactor: &Redactor,
) -> Result<StepOutput, Box<HelperError>> {
    let transport = Transport {
        frames: Mutex::new(VecDeque::from(received)),
        receive_calls: AtomicU32::new(0),
        sent: Mutex::new(Vec::new()),
        send_error: None,
        stream_error: None,
    };
    let mut messages = ClientMessages {
        client: &transport,
        initial,
        redactor,
        done: false,
    };
    let runtime = current_runtime()?;
    let mut collected = Vec::new();
    while let Some(message) = runtime.block_on(messages.next()) {
        collected.push(message);
    }
    let after_end = runtime.block_on(messages.next());
    Ok(StepOutput::Other(json!({
        "messages": collected,
        "receive_calls": transport.receive_calls.load(Ordering::SeqCst),
        "after_end": after_end,
    })))
}

pub(super) fn pending(scope: &EventScope) -> StepOutput {
    let (pending, status, message) = match scope.pending_session_update() {
        Ok(pending) => (pending, None, None),
        Err(error) => (None, Some(error.status_code), Some(error.message)),
    };
    StepOutput::Other(json!({ "pending": pending, "status": status, "message": message }))
}

pub(super) fn provider(
    class_name: &str,
    message: &str,
    scope: &EventScope,
    redactor: &Redactor,
) -> StepOutput {
    let failure = ProviderFailure {
        class_name: class_name.to_owned(),
        message: message.to_owned(),
        source: None,
    };
    let mut scope = scope.clone();
    let error = raise_provider_failure(&mut scope, failure, redactor);
    let (status, message, unchanged) = match error {
        ProviderStreamError::Bedrock {
            message,
            status_code,
            ..
        } => (Some(status_code), message, false),
        ProviderStreamError::Unchanged(failure) => (None, failure.message, true),
    };
    StepOutput::Other(json!({
        "status": status,
        "message": message,
        "recorded": scope.committed_failure,
        "unchanged": unchanged,
    }))
}

pub(super) fn ack(
    ack: &AckStep,
    ids: &dyn IdSource,
    redactor: &Redactor,
) -> Result<StepOutput, Box<HelperError>> {
    let transport = Transport {
        frames: Mutex::new(VecDeque::new()),
        receive_calls: AtomicU32::new(0),
        sent: Mutex::new(Vec::new()),
        send_error: ack.send_error.clone(),
        stream_error: ack.stream_error.clone(),
    };
    let mut scope = ack.scope.clone();
    let mut session = SessionAck {
        client: &transport,
        stream: &transport,
        scope: &mut scope,
        model: MODEL,
        trace_id: ack.trace_id.as_deref(),
        message: &ack.client_message,
        ids,
        redactor,
    };
    let result = current_runtime()?.block_on(session.ack_session_update());
    let (acked, raised) = outcome(result);
    let sent = sent_payload(&transport)?;
    Ok(StepOutput::Other(json!({
        "acked": acked,
        "raised": raised,
        "scope": scope,
        "sent": sent,
    })))
}

fn outcome(result: Result<bool, AckError>) -> (Option<bool>, Option<&'static str>) {
    match result {
        Ok(acked) => (Some(acked), None),
        Err(AckError::Stream(_)) => (None, Some("stream")),
        Err(AckError::Modalities) => (None, Some("modalities")),
        Err(AckError::Encode { .. }) => (None, Some("encode")),
    }
}

fn sent_payload(transport: &Transport) -> Result<Option<serde_json::Value>, Box<HelperError>> {
    transport
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .map(|text| serde_json::from_str::<serde_json::Value>(text))
        .transpose()
        .map_err(failure)
}

pub(super) fn logged_types(configured: &serde_json::Value) -> LoggedEventTypes {
    match configured {
        serde_json::Value::String(star) if star.as_str() == "*" => LoggedEventTypes::All,
        serde_json::Value::Array(items) => LoggedEventTypes::Only(
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_)
        | serde_json::Value::Object(_) => LoggedEventTypes::Default,
    }
}
