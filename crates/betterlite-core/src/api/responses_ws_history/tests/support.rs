use std::fmt::Debug;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

use crate::api::responses_stream_events::ProjectQuotaCallback;
use crate::api::responses_ws_history::error::FrameSinkError;
use crate::api::responses_ws_history::provider::{ProviderResolution, ProviderResolutionFailure};
use crate::api::responses_ws_history::serialization::{ChunkWriter, serialize_chunk};
use crate::api::responses_ws_history::{Connection, FrameSink, ManagedResponsesWebSocket, WsDeps};

pub(super) const MODEL: &str = "openai/gpt-4o-mini";
pub(super) const OTHER_MODEL: &str = "anthropic/claude-3-5-sonnet-20241022";
pub(super) const UNKNOWN_MODEL: &str = "not-a-real-provider/model";
pub(super) const WARMUP_ID_HEX: &str = "0123456789abcdef0123456789abcdef";
pub(super) const NOW: i64 = 1_765_000_000;
pub(super) const ENCODED_RAW_ONE: &str = "resp_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDpncHQtNG8tbWluaTtyZXNwb25zZV9pZDpyYXctMQ==";

pub(super) type Frames = Arc<Mutex<Vec<String>>>;

pub(super) struct RecordingSink {
    frames: Frames,
    fail_next: Arc<AtomicBool>,
}

impl RecordingSink {
    fn push(&self, frame: &str) -> Result<(), FrameSinkError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(FrameSinkError {
                message: "the frame was refused".to_owned(),
            });
        }
        self.frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frame.to_owned());
        Ok(())
    }
}

#[async_trait]
impl FrameSink for RecordingSink {
    async fn send_text(&self, frame: &str) -> Result<(), FrameSinkError> {
        self.push(frame)
    }
}

pub(super) struct Handles {
    pub sink: RecordingSink,
    pub frames: Frames,
    pub fail_next: Arc<AtomicBool>,
}

pub(super) fn recording_sink() -> Handles {
    let frames = Arc::new(Mutex::new(Vec::new()));
    let fail_next = Arc::new(AtomicBool::new(false));
    Handles {
        sink: RecordingSink {
            frames: Arc::clone(&frames),
            fail_next: Arc::clone(&fail_next),
        },
        frames,
        fail_next,
    }
}

pub(super) fn sent(frames: &Frames) -> Vec<String> {
    let recorded = frames.lock().unwrap_or_else(PoisonError::into_inner);
    recorded.clone()
}

pub(super) fn json(text: &str) -> Json {
    let parsed = serde_json::from_str::<Json>(text);
    let Ok(value) = parsed else {
        panic!("the fixture text is not JSON: {text}");
    };
    value
}

pub(super) struct ScriptedResolver;

impl ProviderResolution for ScriptedResolver {
    fn resolve(&self, model: &str) -> Result<String, ProviderResolutionFailure> {
        match model.split_once('/').map(|(provider, _rest)| provider) {
            Some("openai") => Ok("openai".to_owned()),
            Some("anthropic") => Ok("anthropic".to_owned()),
            _ => Err(ProviderResolutionFailure {
                message: format!("no provider answers {model}"),
            }),
        }
    }
}

pub(super) fn fixed_clock(seconds: i64) -> Arc<dyn Fn() -> i64 + Send + Sync> {
    Arc::new(move || seconds)
}

pub(super) fn fixed_ids(id: &str) -> Arc<dyn Fn() -> String + Send + Sync> {
    let fixed = id.to_owned();
    Arc::new(move || fixed.clone())
}

pub(super) fn serializer() -> Arc<ChunkWriter> {
    Arc::new(serialize_chunk)
}

pub(super) fn handler(
    connection: Connection<RecordingSink>,
    resolver: Arc<ScriptedResolver>,
    serializer: Arc<ChunkWriter>,
) -> ManagedResponsesWebSocket<RecordingSink> {
    ManagedResponsesWebSocket::new(
        connection,
        WsDeps {
            resolver,
            clock: fixed_clock(NOW),
            ids: fixed_ids(WARMUP_ID_HEX),
            serializer,
        },
    )
}

pub(super) fn plain(
    connection: Connection<RecordingSink>,
) -> ManagedResponsesWebSocket<RecordingSink> {
    let resolver = Arc::new(ScriptedResolver);
    handler(connection, resolver, serializer())
}

pub(super) fn connection(model: &str, sink: RecordingSink) -> Connection<RecordingSink> {
    Connection {
        websocket: sink,
        model: model.to_owned(),
        logging_obj: logging_object(),
        user_api_key_dict: None,
        litellm_metadata: None,
        api_key: None,
        api_base: None,
        timeout: None,
        custom_llm_provider: None,
        first_message: None,
        quota_callbacks: None,
        kwargs: IndexMap::new(),
    }
}

pub(super) fn key(secret: &str) -> SecretString {
    SecretString::from(secret.to_owned())
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) struct TestQuota;

#[async_trait]
impl ProjectQuotaCallback for TestQuota {
    async fn enforce_frame_quota(
        &self,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        _requested_model: Option<&str>,
        _estimated_input_tokens: i64,
        _estimated_output_tokens: i64,
    ) {
    }
}

const LOGGING_INIT: &str = r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

fn logging_object() -> Arc<StreamLoggingObject> {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        serde_json::from_str::<LoggingInit>(LOGGING_INIT).expect("the logging fixture is valid"),
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the fixture redactor builds"),
        || String::from("trace"),
        Duration::default,
    )
    .expect("the fixture logging state builds");
    Arc::new(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    })
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    pub(super) fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

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

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<T, F>(call: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}
