use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_core::routes::catalog::{RouteCatalog, RouteEntry};
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::stream_item_value;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, StreamingScanKey, TranslationCall,
};
use betterlite_guardrails::unified_dispatch::registry::{TranslationEntry, TranslationRegistry};
use betterlite_guardrails::unified_dispatch::stream_items::chunk_has_finish_reason;
use futures_util::StreamExt as _;
use futures_util::stream;
use thiserror::Error;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::guardrails::unified_stream::StreamOutput;
use crate::guardrails::unified_stream_hook::error::StreamHookError;
use crate::guardrails::unified_stream_hook::run::run_streaming_iterator_hook;
use crate::guardrails::unified_stream_hook::stream_end::StreamEndCheck;
use crate::guardrails::unified_stream_hook::{HookGuardrail, StreamFlagSources, StreamHookParams};

pub(crate) const NAME: &str = "stream-guard";

pub(crate) struct FakeGuardrail;

impl GuardrailContract for FakeGuardrail {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyMode {
    Texts,
    Missing,
    ToolsInFlight,
}

// The length of responses_so_far at each scan, and whether the incremental loop's sink came along
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScanCall {
    pub(crate) items: usize,
    pub(crate) sink: bool,
}

pub(crate) struct FakeTranslation {
    plans: Mutex<VecDeque<Result<(), GuardrailFailure>>>,
    calls: Mutex<Vec<ScanCall>>,
    key: KeyMode,
    error_items: Option<Vec<StreamItem>>,
}

impl FakeTranslation {
    pub(crate) fn calls(&self) -> Vec<ScanCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) fn content(item: &GuardrailValue) -> String {
    if let GuardrailValue::Text(text) = item {
        return text.clone();
    }
    let GuardrailValue::Object(fields) = item else {
        return String::from("?");
    };
    let Some(GuardrailValue::Array(choices)) = fields.get("choices") else {
        return String::from("?");
    };
    let Some(GuardrailValue::Object(choice)) = choices.first() else {
        return String::from("?");
    };
    let Some(GuardrailValue::Object(delta)) = choice.get("delta") else {
        return String::from("?");
    };
    match delta.get("content") {
        Some(GuardrailValue::Text(text)) => text.clone(),
        Some(_) | None => String::from("?"),
    }
}

fn joined(items: &[GuardrailValue]) -> String {
    items.iter().map(content).collect::<String>()
}

fn joined_items(items: &[StreamItem]) -> String {
    let values: Vec<GuardrailValue> = items.iter().map(stream_item_value).collect();
    joined(&values)
}

impl BaseTranslation for FakeTranslation {
    type Guardrail = FakeGuardrail;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    fn process_input_messages(
        &self,
        _data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        ready(Ok(response))
    }

    fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<StreamItem>,
        _guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ScanCall {
                items: responses_so_far.len(),
                sink: call.stream_transform_sink.is_some(),
            });
        let plan = self
            .plans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(plan.unwrap_or(Ok(())))
    }

    fn get_streaming_scan_key(&self, responses_so_far: &[StreamItem]) -> Option<StreamingScanKey> {
        let in_flight = match self.key {
            KeyMode::Missing => return None,
            KeyMode::Texts => false,
            KeyMode::ToolsInFlight => true,
        };
        Some(StreamingScanKey {
            texts: vec![joined_items(responses_so_far)],
            tool_calls: Vec::new(),
            stream_ended: false,
            tool_calls_in_flight: in_flight,
        })
    }

    fn build_block_sse_chunks(
        &self,
        failure: &Self::Failure,
        stream_started: bool,
        responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<Vec<u8>>> {
        let GuardrailFailure::ModifyResponse(details) = failure else {
            return None;
        };
        if details.message == "unwritable" {
            return None;
        }
        let original = match &details.original_response {
            Some(GuardrailValue::Array(items)) => joined(items),
            Some(other) => content(other),
            None => String::from("none"),
        };
        let sent = responses_so_far.unwrap_or_default();
        let summary = format!(
            "block started={stream_started} sent={}:{} original={original}",
            sent.len(),
            joined_items(sent)
        );
        Some(vec![summary.into_bytes()])
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<StreamItem>> {
        if let Some(items) = &self.error_items {
            return Some(items.clone());
        }
        let status = match failure {
            GuardrailFailure::Http { status, .. } | GuardrailFailure::HttpObject { status, .. } => {
                *status
            }
            GuardrailFailure::ModifyResponse(_)
            | GuardrailFailure::GuardrailRaised { .. }
            | GuardrailFailure::BlockedPii { .. }
            | GuardrailFailure::SensitiveDataRoute(_)
            | GuardrailFailure::Other(_) => return None,
        };
        let sent = joined_items(responses_so_far.unwrap_or_default());
        Some(vec![StreamItem::Json(GuardrailValue::Text(format!(
            "error status={status} sent={sent}"
        )))])
    }
}

impl StreamEndCheck for FakeTranslation {
    // the fake probes only the last chunk, like the responses translation, so a pending chunk can outlive the end
    fn streaming_has_ended(&self, items: &[StreamItem]) -> bool {
        let Some(StreamItem::Json(last)) = items.last() else {
            return false;
        };
        chunk_has_finish_reason(last)
    }
}

pub(crate) fn chunk(text: &str, finish_reason: Option<&str>) -> GuardrailValue {
    let reason = finish_reason.map_or(GuardrailValue::Null, |given| {
        GuardrailValue::Text(given.to_owned())
    });
    let delta = BTreeMap::from([(
        String::from("content"),
        GuardrailValue::Text(text.to_owned()),
    )]);
    let choice = BTreeMap::from([
        (String::from("index"), GuardrailValue::SignedInteger(0)),
        (String::from("delta"), GuardrailValue::Object(delta)),
        (String::from("finish_reason"), reason),
    ]);
    GuardrailValue::Object(BTreeMap::from([
        (
            String::from("object"),
            GuardrailValue::Text(String::from("chat.completion.chunk")),
        ),
        (
            String::from("choices"),
            GuardrailValue::Array(vec![GuardrailValue::Object(choice)]),
        ),
    ]))
}

pub(crate) fn texts(names: &str) -> Vec<GuardrailValue> {
    names
        .chars()
        .map(|name| chunk(&name.to_string(), None))
        .collect()
}

pub(crate) fn modify_response(message: &str, original: Option<GuardrailValue>) -> GuardrailFailure {
    GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: message.to_owned(),
        model: String::from("gpt-4o"),
        request_data: BTreeMap::new(),
        guardrail_name: Some(NAME.to_owned()),
        detection_info: BTreeMap::new(),
        original_response: original,
    }))
}

// The failure a test source raises where the provider would have dropped the stream
#[derive(Debug, PartialEq, Eq, Error)]
#[error("the provider dropped the stream")]
pub(crate) struct SourceFailure;

pub(crate) type Outputs = Vec<Result<StreamOutput, StreamHookError<SourceFailure>>>;

// Each output as one line: the chunk text, the block summary, an error item or the error variant
pub(crate) fn lines(outputs: &Outputs) -> Vec<String> {
    outputs
        .iter()
        .map(|output| match output {
            Ok(StreamOutput::Item(StreamItem::Json(GuardrailValue::Text(text)))) => text.clone(),
            Ok(StreamOutput::Item(StreamItem::Json(item))) => content(&item),
            Ok(StreamOutput::Item(StreamItem::Bytes(bytes))) => {
                format!("bytes {}", String::from_utf8(bytes.clone()).unwrap())
            }
            Ok(StreamOutput::BlockChunks(chunks)) => String::from_utf8(chunks.concat()).unwrap(),
            Err(StreamHookError::InvalidFlag(invalid)) => format!("invalid {}", invalid.name),
            Err(StreamHookError::Gate { .. }) => String::from("gate error"),
            Err(StreamHookError::Loop(error)) => format!("loop error {error}"),
        })
        .collect()
}

pub(crate) fn settings() -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some(NAME.to_owned()),
        supported_event_hooks: None,
        event_hook: None,
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches::default(),
    }
}

pub(crate) struct Harness {
    pub(crate) base: GuardrailBase,
    pub(crate) registry: TranslationRegistry<FakeTranslation>,
    pub(crate) routes: RouteCatalog,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
    pub(crate) flags: StreamFlagSources,
    pub(crate) route: Option<&'static str>,
    pub(crate) buffer_default: bool,
    pub(crate) guarded: bool,
}

fn fake(key: KeyMode, plans: Vec<Result<(), GuardrailFailure>>) -> FakeTranslation {
    FakeTranslation {
        plans: Mutex::new(VecDeque::from(plans)),
        calls: Mutex::new(Vec::new()),
        key,
        error_items: None,
    }
}

fn route(path: &str, call_type: CallType) -> RouteEntry {
    RouteEntry {
        route: path.to_owned(),
        call_types: vec![call_type],
    }
}

impl Harness {
    pub(crate) fn new(key: KeyMode, plans: Vec<Result<(), GuardrailFailure>>) -> Self {
        let entries = vec![
            TranslationEntry {
                call_type: CallType::Completion,
                translation: fake(key, plans),
                chat_completions: true,
            },
            TranslationEntry {
                call_type: CallType::Responses,
                translation: fake(key, Vec::new()),
                chat_completions: false,
            },
            TranslationEntry {
                call_type: CallType::AnthropicMessages,
                translation: fake(key, Vec::new()),
                chat_completions: false,
            },
        ];
        Self {
            base: GuardrailBase {
                settings: settings(),
                mode: None,
            },
            registry: TranslationRegistry { entries },
            routes: RouteCatalog {
                source: String::new(),
                call_types: Vec::new(),
                routes: vec![
                    route("/chat", CallType::Completion),
                    route("/responses", CallType::Responses),
                    route("/v1/messages", CallType::AnthropicMessages),
                    route("/embed", CallType::Embedding),
                    RouteEntry {
                        route: String::from("/empty"),
                        call_types: Vec::new(),
                    },
                ],
            },
            request_data: BTreeMap::new(),
            flags: StreamFlagSources::default(),
            route: None,
            buffer_default: false,
            guarded: true,
        }
    }

    pub(crate) fn flag(mut self, name: &str, value: GuardrailValue) -> Self {
        self.flags.optional_params.insert(name.to_owned(), value);
        self
    }

    pub(crate) fn rate(self, rate: i64) -> Self {
        self.flag(
            "streaming_sampling_rate",
            GuardrailValue::SignedInteger(rate),
        )
    }

    pub(crate) fn calls(&self, call_type: CallType) -> Vec<ScanCall> {
        self.registry.get(call_type).unwrap().translation.calls()
    }

    pub(crate) fn error_items(mut self, items: Vec<StreamItem>) -> Self {
        for entry in &mut self.registry.entries {
            entry.translation.error_items = Some(items.clone());
        }
        self
    }

    pub(crate) fn plans(
        mut self,
        call_type: CallType,
        plans: Vec<Result<(), GuardrailFailure>>,
    ) -> Self {
        if let Some(entry) = self
            .registry
            .entries
            .iter_mut()
            .find(|entry| entry.call_type == call_type)
        {
            *entry
                .translation
                .plans
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = VecDeque::from(plans);
        }
        self
    }

    pub(crate) async fn run(&mut self, items: Vec<GuardrailValue>) -> Outputs {
        let items = items.into_iter().map(StreamItem::Json);
        self.run_frames_source(items.map(Ok).collect()).await
    }

    pub(crate) async fn run_frames(&mut self, items: Vec<StreamItem>) -> Outputs {
        self.run_frames_source(items.into_iter().map(Ok).collect())
            .await
    }

    pub(crate) async fn run_source(
        &mut self,
        source: Vec<Result<GuardrailValue, SourceFailure>>,
    ) -> Outputs {
        let items = source
            .into_iter()
            .map(|result| result.map(StreamItem::Json));
        self.run_frames_source(items.collect()).await
    }

    pub(crate) async fn run_frames_source(
        &mut self,
        source: Vec<Result<StreamItem, SourceFailure>>,
    ) -> Outputs {
        let user_api_key_dict = BTreeMap::new();
        let params = StreamHookParams {
            guardrail: self.guarded.then_some(HookGuardrail {
                guardrail: &FakeGuardrail,
                base: &self.base,
            }),
            registry: &self.registry,
            routes: &self.routes,
            request_data: &mut self.request_data,
            user_api_key_dict: &user_api_key_dict,
            request_route: self.route,
            flags: self.flags.clone(),
            buffer_until_moderated_default: self.buffer_default,
        };
        run_streaming_iterator_hook(params, stream::iter(source))
            .collect()
            .await
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
