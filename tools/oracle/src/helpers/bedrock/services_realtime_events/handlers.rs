use std::collections::VecDeque;
use std::io;
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
use tokio::runtime::{Builder, Runtime};

use super::{
    AckOutput, AckStep, AllEventTypes, ClientMessagesOutput, ConfiguredEventTypes, MODEL,
    PendingOutput, ProviderOutput, SentSessionEvent, failure,
};
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
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        self.receive_calls.fetch_add(1, Ordering::SeqCst);
        let received = self
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .flatten();
        let Some(frame) = received else {
            return Err(ClientError {
                message: "the client sent no more frames".to_owned(),
                source: None,
            });
        };
        Ok(frame)
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError<io::Error>> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data);
        if let Some(message) = &self.send_error {
            return Err(ClientError {
                message: message.clone(),
                source: None,
            });
        }
        Ok(())
    }

    async fn close(&self, _code: u16, _reason: Option<String>) {}
}

#[async_trait]
impl BedrockInputStream for Transport {
    type Error = io::Error;

    async fn send(&self, _event: Json) -> Result<(), StreamError<io::Error>> {
        Ok(())
    }

    async fn close(&self) -> Result<(), StreamError<io::Error>> {
        Ok(())
    }
}

struct EmptyOutput;

#[async_trait]
impl BedrockOutputStream for EmptyOutput {
    type Error = io::Error;

    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<io::Error>> {
        Ok(None)
    }
}

#[async_trait]
impl BedrockBidirectionalStream for Transport {
    type Error = io::Error;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = io::Error> {
        self
    }

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = io::Error>>), StreamError<io::Error>>
    {
        if let Some(message) = &self.stream_error {
            return Err(StreamError {
                message: message.clone(),
                source: None,
            });
        }
        Ok((Json::Null, Box::new(EmptyOutput)))
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
) -> Result<ClientMessagesOutput, Box<HelperError>> {
    let transport = Transport {
        frames: Mutex::new(VecDeque::from(received)),
        receive_calls: AtomicU32::new(0),
        sent: Mutex::new(Vec::new()),
        send_error: None,
        stream_error: None,
    };
    let mut messages: ClientMessages<'_, io::Error> = ClientMessages {
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
    Ok(ClientMessagesOutput {
        messages: collected,
        receive_calls: transport.receive_calls.load(Ordering::SeqCst),
        after_end,
    })
}

pub(super) fn pending(scope: &EventScope) -> PendingOutput {
    match scope.pending_session_update() {
        Ok(pending) => PendingOutput {
            pending,
            status: None,
        },
        Err(error) => PendingOutput {
            pending: None,
            status: Some(error.status_code),
        },
    }
}

pub(super) fn provider(
    class_name: &str,
    message: &str,
    scope: &EventScope,
    redactor: &Redactor,
) -> ProviderOutput {
    let failure: ProviderFailure<io::Error> = ProviderFailure {
        class_name: class_name.to_owned(),
        message: message.to_owned(),
        source: None,
    };
    let mut updated_scope = scope.clone();
    let error = raise_provider_failure(&mut updated_scope, failure, redactor);
    let (status, unchanged) = match error {
        ProviderStreamError::Bedrock { status_code, .. } => (Some(status_code), false),
        ProviderStreamError::Unchanged(_) => (None, true),
    };
    ProviderOutput {
        status,
        recorded: !matches!(updated_scope.committed_failure, Json::Null),
        unchanged,
    }
}

pub(super) fn ack(
    ack: &AckStep,
    ids: &dyn IdSource,
    redactor: &Redactor,
) -> Result<AckOutput, Box<HelperError>> {
    let transport = Transport {
        frames: Mutex::new(VecDeque::new()),
        receive_calls: AtomicU32::new(0),
        sent: Mutex::new(Vec::new()),
        send_error: ack.send_error.clone(),
        stream_error: ack.stream_error.clone(),
    };
    let mut scope = ack.scope.clone();
    let mut session: SessionAck<'_, io::Error, io::Error> = SessionAck {
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
    let (acked, raised) = outcome(&result);
    Ok(AckOutput {
        acked,
        raised,
        scope,
        sent: sent_event(&transport)?,
    })
}

const fn outcome(
    result: &Result<bool, AckError<io::Error>>,
) -> (Option<bool>, Option<&'static str>) {
    match result {
        Ok(acked) => (Some(*acked), None),
        Err(AckError::Stream(_)) => (None, Some("stream")),
        Err(AckError::Modalities) => (None, Some("modalities")),
        Err(AckError::Encode { .. }) => (None, Some("encode")),
    }
}

fn sent_event(transport: &Transport) -> Result<Option<SentSessionEvent>, Box<HelperError>> {
    transport
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .map(|text| serde_json::from_str::<SentSessionEvent>(text))
        .transpose()
        .map_err(failure)
}

pub(super) fn logged_types(configured: Option<&ConfiguredEventTypes>) -> LoggedEventTypes {
    match configured {
        None => LoggedEventTypes::Default,
        Some(ConfiguredEventTypes::All(AllEventTypes::Star)) => LoggedEventTypes::All,
        Some(ConfiguredEventTypes::Only(types)) => LoggedEventTypes::Only(types.clone()),
    }
}
