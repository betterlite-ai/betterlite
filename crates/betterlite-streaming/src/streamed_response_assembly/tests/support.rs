use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use rust_decimal::Decimal;

use crate::stream_base_assembly::provider_metadata::{
    ChatConfigSelection, ProjectionError, ProviderMetadataProjection, ProviderResolution,
};
use crate::stream_base_assembly::records::Chunk as BaseChunk;
use crate::stream_base_assembly::response::BaseResponse;
use crate::stream_chunk_shaping::records::JsonValue;
use crate::stream_usage_assembly::UsageTokenCounter;
use crate::stream_usage_assembly::wire::Usage;
use crate::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator, StreamLogging,
};
use crate::streamed_response_assembly::records::Chunk;

#[derive(Default)]
pub(super) struct RecordingLogger {
    records: Mutex<Vec<Payload>>,
}

impl RecordingLogger {
    fn record(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .expect("fixture verbose log")
            .push(payload.clone());
        true
    }

    pub(super) fn messages(&self) -> Vec<String> {
        self.records
            .lock()
            .expect("fixture verbose log")
            .iter()
            .map(|payload| match payload {
                Payload::String(text) => text.clone(),
                other @ (Payload::Null
                | Payload::Bool(_)
                | Payload::Number(_)
                | Payload::Array(_)
                | Payload::Object(_)) => other.to_string(),
            })
            .collect()
    }
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.record(payload)
    }

    fn info(&self, payload: &Payload) -> bool {
        self.record(payload)
    }

    fn error(&self, payload: &Payload) -> bool {
        self.record(payload)
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

#[derive(Default)]
pub(super) struct RecordingConsole;

impl ConsoleSink for RecordingConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

pub(super) fn redactor() -> Arc<Redactor> {
    Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles"))
}

pub(super) fn hooks(redactor: &Arc<Redactor>) -> VerboseHooks<RecordingLogger, RecordingConsole> {
    VerboseHooks {
        logger: RecordingLogger::default(),
        console: RecordingConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::clone(redactor),
    }
}

#[derive(Default)]
pub(super) struct FixtureCounter {
    pub(super) response_text: i64,
    pub(super) text: i64,
    pub(super) messages: i64,
    pub(super) fail_response_text: bool,
    pub(super) fail_messages: bool,
    pub(super) seen_completion: Mutex<Option<String>>,
    pub(super) seen_reasoning: Mutex<Option<String>>,
    pub(super) seen_model: Mutex<Option<String>>,
}

impl FixtureCounter {
    const fn counted(failed: bool, count: i64) -> Result<i64, CountError> {
        if failed {
            return Err(CountError::MissingInput);
        }
        Ok(count)
    }

    pub(super) fn seen_completion(&self) -> Option<String> {
        self.seen_completion
            .lock()
            .expect("fixture counter completion text")
            .clone()
    }

    pub(super) fn seen_reasoning(&self) -> Option<String> {
        self.seen_reasoning
            .lock()
            .expect("fixture counter reasoning text")
            .clone()
    }

    pub(super) fn seen_model(&self) -> Option<String> {
        self.seen_model
            .lock()
            .expect("fixture counter model")
            .clone()
    }
}

#[async_trait::async_trait]
impl UsageTokenCounter for FixtureCounter {
    async fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError> {
        *self
            .seen_completion
            .lock()
            .expect("fixture counter completion text") = Some(text.to_owned());
        *self.seen_model.lock().expect("fixture counter model") = Some(model.to_owned());
        Self::counted(self.fail_response_text, self.response_text)
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        *self
            .seen_reasoning
            .lock()
            .expect("fixture counter reasoning text") = Some(text.to_owned());
        Self::counted(self.fail_response_text, self.text)
    }

    async fn count_messages(
        &self,
        _model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        if messages.is_none() {
            return Err(CountError::MissingInput);
        }
        Self::counted(self.fail_messages, self.messages)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum ProviderSeen {
    #[default]
    NotCalled,
    Named(Option<String>),
}

#[derive(Default)]
pub(super) struct FixtureCosts {
    pub(super) completion: Option<Decimal>,
    pub(super) completion_error: bool,
    pub(super) prompt: Option<Decimal>,
    pub(super) completion_tokens: Option<Decimal>,
    pub(super) per_token_error: bool,
    pub(super) seen_provider: Mutex<ProviderSeen>,
    pub(super) completion_calls: AtomicUsize,
}

impl FixtureCosts {
    pub(super) fn seen_provider(&self) -> ProviderSeen {
        self.seen_provider
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn completion_calls(&self) -> usize {
        self.completion_calls.load(Ordering::SeqCst)
    }
}

const COMPLETION_FAILURE: &str = "fixture completion cost failure";
const LOGGING_FAILURE: &str = "fixture logging cost failure";

// Both price fixtures count the call, then fail or answer with the fixed price.
fn priced(
    calls: &AtomicUsize,
    fail: bool,
    message: &str,
    price: Option<Decimal>,
) -> Result<Option<Decimal>, CostError> {
    calls.fetch_add(1, Ordering::SeqCst);
    if fail {
        return Err(CostError {
            message: message.to_owned(),
        });
    }
    Ok(price)
}

impl StreamCostCalculator for FixtureCosts {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        *self
            .seen_provider
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            ProviderSeen::Named(provider.map(str::to_owned));
        priced(
            &self.completion_calls,
            self.completion_error,
            COMPLETION_FAILURE,
            self.completion,
        )
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        if self.per_token_error {
            return Err(CostError {
                message: "fixture per token cost failure".to_owned(),
            });
        }
        Ok((
            self.prompt.unwrap_or_default(),
            self.completion_tokens.unwrap_or_default(),
        ))
    }
}

#[derive(Default)]
pub(super) struct FixtureLogging {
    pub(super) provider: Option<&'static str>,
    pub(super) cost: Option<Decimal>,
    pub(super) fail: bool,
    pub(super) calls: AtomicUsize,
}

impl FixtureLogging {
    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl StreamLogging for FixtureLogging {
    fn provider(&self) -> Option<&str> {
        self.provider
    }

    fn response_cost(&self, _input: &CostInput<'_>) -> Result<Option<Decimal>, CostError> {
        priced(&self.calls, self.fail, LOGGING_FAILURE, self.cost)
    }
}

// A logging fixture that names its provider and answers every cost call with the same price.
pub(super) fn priced_logging(provider: &'static str, cost: Option<Decimal>) -> FixtureLogging {
    FixtureLogging {
        provider: Some(provider),
        cost,
        fail: false,
        calls: AtomicUsize::new(0),
    }
}

pub(super) fn failing_logging(provider: &'static str) -> FixtureLogging {
    FixtureLogging {
        provider: Some(provider),
        cost: None,
        fail: true,
        calls: AtomicUsize::new(0),
    }
}

pub(super) struct FixtureResolver;

impl ProviderResolution for FixtureResolver {
    fn provider_for_model(&self, _model: &str) -> Result<ProviderId, ResolutionError> {
        Ok(ProviderId::Openai)
    }
}

pub(super) struct FixtureSelector;

impl ChatConfigSelection for FixtureSelector {
    fn select_chat_config(
        &self,
        _model: &str,
        _provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        Ok(Some(ChatConfigTag::OpenAiGpt))
    }
}

#[derive(Default)]
pub(super) struct RecordingProjection {
    pub(super) seen_cost: Mutex<Option<JsonValue>>,
}

impl RecordingProjection {
    pub(super) fn seen_cost(&self) -> Option<JsonValue> {
        self.seen_cost
            .lock()
            .expect("fixture projection cost")
            .clone()
    }
}

impl ProviderMetadataProjection for RecordingProjection {
    fn project(
        &self,
        tag: ChatConfigTag,
        response: &mut BaseResponse,
        _chunks: &[BaseChunk],
    ) -> Result<(), ProjectionError> {
        *self
            .seen_cost
            .lock()
            .unwrap_or_else(PoisonError::into_inner) =
            response.hidden_params.get("response_cost").cloned();
        response.hidden_params.insert(
            "projected".to_owned(),
            JsonValue::String(tag.class_name().to_owned()),
        );
        Ok(())
    }
}

pub(super) fn chunks(raw: &str) -> Vec<Chunk> {
    serde_json::from_str(raw).expect("fixture chunks parse")
}
