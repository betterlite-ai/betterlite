use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::exporter_options::redaction::ResponseStamp;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_failure::handlers::{SyncFailureJob, run_sync_failure};
use betterlite_logging::logging_failure::{
    FailureClock, FailureDeps, FailurePayloadInput, FailurePayloadSource, SyncAdmission,
    SyncFailureExecutor,
};
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::identity::CallbackIdentity;
use betterlite_logging::registry::phases::CallbackList;
use betterlite_logging::registry::sink::CallbackSink;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::input::Instance;
use crate::compare::helper::HelperError;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LoggerFlag {
    TurnOffMessageLogging,
    RedactsMessagesItself,
    FailSync,
    FailAsync,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoggerSpec {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) flags: Vec<LoggerFlag>,
}

pub(crate) struct RecordedCall {
    pub(crate) logger: String,
    pub(crate) method: &'static str,
    pub(crate) stream: bool,
}

pub(crate) struct Probe {
    spec: LoggerSpec,
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl Probe {
    fn record(&self, method: &'static str, event: &LoggerEvent) -> Result<(), LoggerError> {
        lock(&self.calls).push(RecordedCall {
            logger: self.spec.name.clone(),
            method,
            stream: event.stream,
        });
        let failing = match method {
            "async" => self.spec.flags.contains(&LoggerFlag::FailAsync),
            _ => self.spec.flags.contains(&LoggerFlag::FailSync),
        };
        if failing {
            return Err(LoggerError::Delivery {
                callback: self.spec.name.clone(),
                message: "the probe refused the event".to_owned(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl CallbackLogger for Probe {
    fn callback_name(&self) -> &str {
        &self.spec.name
    }

    fn turn_off_message_logging(&self) -> bool {
        self.spec.flags.contains(&LoggerFlag::TurnOffMessageLogging)
    }

    fn redacts_messages_itself(&self) -> bool {
        self.spec.flags.contains(&LoggerFlag::RedactsMessagesItself)
    }

    fn log_failure_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        self.record("sync", event)
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record("async", &event)
    }
}

pub(crate) struct Fakes {
    pub(crate) registry: CallbackRegistry,
    pub(crate) calls: Arc<Mutex<Vec<RecordedCall>>>,
    payload: Option<StandardLoggingPayload>,
    pub(crate) jobs: AtomicUsize,
    clock: DateTime<FixedOffset>,
}

impl Fakes {
    pub(crate) fn new(instance: &Instance) -> Result<Self, Box<HelperError>> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut registry = CallbackRegistry::new(100);
        for spec in &instance.loggers {
            let probe = Arc::new(Probe {
                spec: spec.clone(),
                calls: Arc::clone(&calls),
            });
            let _registration = registry.register(
                CallbackIdentity {
                    class_name: spec.name.clone(),
                    attributes: IndexMap::new(),
                },
                CallbackSink::Logger(Arc::<Probe>::clone(&probe)),
                CallbackList::Failure,
            );
        }
        Ok(Self {
            registry,
            calls,
            payload: decode_payload(instance.payload.as_ref())?,
            jobs: AtomicUsize::new(0),
            clock: instance.clock.unwrap_or(instance.init.start_time),
        })
    }

    pub(crate) fn deps<'deps>(&'deps self, redactor: &'deps Redactor) -> FailureDeps<'deps> {
        FailureDeps {
            redactor,
            payloads: self,
            clock: self,
            executor: self,
            baseline: None,
            flush: None,
            special_listeners: &[],
            counters: Vec::new(),
            excluded_fields: &[],
            stamp: ResponseStamp {
                id: "chatcmpl-probe".to_owned(),
                created: 0,
            },
            privacy: Settings::default(),
        }
    }

    pub(crate) fn calls_since(&self, start: usize) -> Value {
        let calls = lock(&self.calls);
        Value::Array(
            calls
                .iter()
                .skip(start)
                .map(|call| {
                    json!({
                        "logger": &call.logger,
                        "method": call.method,
                        "stream": call.stream,
                    })
                })
                .collect(),
        )
    }
}

impl FailureClock for Fakes {
    fn now(&self) -> DateTime<FixedOffset> {
        self.clock
    }
}

impl FailurePayloadSource for Fakes {
    fn build(&self, _input: FailurePayloadInput<'_>) -> Option<StandardLoggingPayload> {
        self.payload.clone()
    }
}

impl SyncFailureExecutor for Fakes {
    fn enqueue_sync_failure(&self, job: SyncFailureJob) -> SyncAdmission {
        self.jobs.fetch_add(1, Ordering::SeqCst);
        run_sync_failure(job);
        SyncAdmission::Accepted
    }
}

fn decode_payload(
    payload: Option<&Value>,
) -> Result<Option<StandardLoggingPayload>, Box<HelperError>> {
    let Some(value) = payload else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}
