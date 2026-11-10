use std::sync::Arc;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::exporter_options::failure_counter::CallbackFailureCounter;
use betterlite_logging::exporter_options::redaction::ResponseStamp;
use betterlite_logging::logging_success::handler::SuccessHandler;
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::Registration;
use betterlite_logging::registry::identity::CallbackIdentity;
use betterlite_logging::registry::phases::CallbackList;
use betterlite_logging::registry::sink::CallbackSink;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde_json::Value;

use crate::compare::helper::HelperError;

use betterlite_logging::fields::payload::StandardLoggingPayload;

use super::input::{LoggerSpec, ResultKind, ResultSpec, StampSpec};
use super::state::decode;
use super::support::{FixedAssembler, FixedBuilder, FixedClock, RecordingCounter, RecordingLogger};

pub(crate) struct ScenarioSpec<'probe> {
    pub(crate) loggers: &'probe [LoggerSpec],
    pub(crate) built_payload: Option<&'probe Value>,
    pub(crate) complete: Option<&'probe Value>,
    pub(crate) now: DateTime<FixedOffset>,
    pub(crate) excluded_fields: Vec<String>,
    pub(crate) stamp: Option<&'probe StampSpec>,
}

pub(crate) struct Scenario {
    pub(crate) registry: CallbackRegistry,
    pub(crate) builder: FixedBuilder,
    pub(crate) assembler: FixedAssembler,
    pub(crate) clock: FixedClock,
    pub(crate) counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    pub(crate) counter_calls: Vec<Arc<RecordingCounter>>,
    pub(crate) loggers: Vec<Arc<RecordingLogger>>,
    pub(crate) excluded_fields: Vec<String>,
    pub(crate) stamp: ResponseStamp,
    pub(crate) known: Box<dyn Fn(&str) -> bool + Send + Sync>,
    pub(crate) privacy: Settings,
}

impl Scenario {
    pub(crate) fn new(spec: ScenarioSpec<'_>) -> Result<Self, Box<HelperError>> {
        let (registry, loggers) = loggers(spec.loggers)?;
        let (counters, counter_calls) = counters();
        Ok(Self {
            registry,
            builder: FixedBuilder {
                payload: spec
                    .built_payload
                    .map(decode::<StandardLoggingPayload>)
                    .transpose()?,
            },
            assembler: FixedAssembler {
                complete: spec.complete.map(decode::<Json>).transpose()?,
                declines_results: false,
            },
            clock: FixedClock { now: spec.now },
            counters,
            counter_calls,
            loggers,
            excluded_fields: spec.excluded_fields,
            stamp: stamp_value(spec.stamp),
            known: Box::new(|_name: &str| false),
            privacy: Settings::default(),
        })
    }

    pub(crate) fn handler(&self) -> SuccessHandler<'_> {
        SuccessHandler {
            registry: &self.registry,
            payload_builder: &self.builder,
            stream_assembler: &self.assembler,
            clock: &self.clock,
            failure_counters: &self.counters,
            excluded_fields: &self.excluded_fields,
            known_callback: self.known.as_ref(),
            stamp: &self.stamp,
            privacy: self.privacy,
        }
    }
}

fn loggers(
    specs: &[LoggerSpec],
) -> Result<(CallbackRegistry, Vec<Arc<RecordingLogger>>), Box<HelperError>> {
    let mut registry = CallbackRegistry::new(64);
    let mut recording = Vec::new();
    for logger in specs {
        let record = Arc::new(RecordingLogger::new(
            &logger.name,
            &logger.flags,
            logger.fail,
        ));
        let identity = CallbackIdentity {
            class_name: logger.name.clone(),
            attributes: IndexMap::new(),
        };
        let concrete = Arc::clone(&record);
        let sink_logger: Arc<dyn CallbackLogger> = concrete;
        let sink = CallbackSink::Logger(sink_logger);
        if registry.register(identity, sink, CallbackList::Callbacks)
            == Registration::SkippedCapacity
        {
            return Err(super::value_error(&"the callback registry is full"));
        }
        recording.push(record);
    }
    Ok((registry, recording))
}

fn counters() -> (
    Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    Vec<Arc<RecordingCounter>>,
) {
    let first = Arc::new(RecordingCounter::default());
    let second = Arc::new(RecordingCounter::default());
    let first_concrete = Arc::clone(&first);
    let first_handle: Arc<dyn CallbackFailureCounter + Send + Sync> = first_concrete;
    let second_concrete = Arc::clone(&second);
    let second_handle: Arc<dyn CallbackFailureCounter + Send + Sync> = second_concrete;
    (vec![first_handle, second_handle], vec![first, second])
}

fn stamp_value(spec: Option<&StampSpec>) -> ResponseStamp {
    spec.map_or_else(
        || ResponseStamp {
            id: "chatcmpl-placeholder".to_owned(),
            created: 1_704_067_200,
        },
        |stamp| ResponseStamp {
            id: stamp.id.clone(),
            created: stamp.created,
        },
    )
}

pub(crate) fn result(spec: &ResultSpec) -> Result<SuccessResult, Box<HelperError>> {
    let value = match &spec.value {
        Some(raw) => decode::<Json>(raw)?,
        None => default_value(spec.kind)?,
    };
    Ok(spec.kind.wrap(value))
}

impl ResultKind {
    const fn wrap(self, value: Json) -> SuccessResult {
        match self {
            Self::Chat => SuccessResult::Chat(value),
            Self::Embeddings => SuccessResult::Embeddings(value),
            Self::Chunk => SuccessResult::Chunk(value),
            Self::Other => SuccessResult::Other(value),
        }
    }
}

fn default_value(kind: ResultKind) -> Result<Json, Box<HelperError>> {
    let text = match kind {
        ResultKind::Chat => r#"{"id":"chatcmpl-1","object":"chat.completion"}"#,
        ResultKind::Embeddings => r#"{"id":"embedding-1","object":"list"}"#,
        ResultKind::Chunk => r#"{"id":"chatcmpl-chunk","object":"chat.completion.chunk"}"#,
        ResultKind::Other => "null",
    };
    let value: Value =
        serde_json::from_str(text).map_err(|failure| super::value_error(&failure))?;
    decode::<Json>(&value)
}

pub(crate) fn default_time() -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
        .map_err(|failure| super::value_error(&failure))
}
