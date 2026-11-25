use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_core::api::responses_stream_events::ProjectQuotaCallback;
use betterlite_core::api::responses_ws_history::serialization::serialize_chunk;
use betterlite_core::api::responses_ws_history::{Connection, ManagedResponsesWebSocket, WsDeps};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use secrecy::SecretString;
use tracing::field::Field;
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer};

use super::Setup;
use super::fakes::{ScriptedQuota, ScriptedResolver};
use super::input::QuotaRow;
use super::support::{Shared, Socket, lock};

const CLOCK_SECONDS: i64 = 1_765_000_000;
const WARMUP_ID_HEX: &str = "0123456789abcdef0123456789abcdef";
const DIAGNOSTIC_TARGET: &str = "betterlite_core::api::responses_ws_";
const DIAGNOSTIC_PREFIX: &str = "ManagedResponsesWS: ";

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) fn build_handler(
    setup: &Setup,
    socket: Socket,
    shared: &Shared,
    quota: Option<&QuotaRow>,
) -> ManagedResponsesWebSocket<Socket> {
    let callbacks: Option<Vec<Arc<dyn ProjectQuotaCallback>>> = quota.map(|row| {
        let callback: Arc<dyn ProjectQuotaCallback> = Arc::new(ScriptedQuota {
            shared: Arc::clone(shared),
            row: row.clone(),
        });
        vec![callback]
    });
    let connection = Connection {
        websocket: socket,
        model: setup.model.clone(),
        logging_obj: Arc::new(StreamLoggingObject {
            slot: Mutex::new(None),
            dispatch: Arc::new(NoDispatch),
        }),
        user_api_key_dict: None,
        litellm_metadata: setup.litellm_metadata.clone(),
        api_key: setup.api_key.clone().map(SecretString::from),
        api_base: setup.api_base.clone(),
        timeout: setup.timeout,
        custom_llm_provider: setup.custom_llm_provider.clone(),
        first_message: setup.first_message.clone(),
        quota_callbacks: callbacks,
        kwargs: setup.kwargs.clone(),
    };
    ManagedResponsesWebSocket::new(
        connection,
        WsDeps {
            resolver: Arc::new(ScriptedResolver),
            clock: Arc::new(|| CLOCK_SECONDS),
            ids: Arc::new(|| WARMUP_ID_HEX.to_owned()),
            serializer: Arc::new(serialize_chunk),
        },
    )
}

#[derive(Clone, Default)]
pub(super) struct Captured(pub(super) Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if !event.metadata().target().starts_with(DIAGNOSTIC_TARGET) {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        if !message.0.starts_with(DIAGNOSTIC_PREFIX) {
            return;
        }
        lock(&self.0).push(message.0);
    }
}
