use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::worker::{
    Admission, Callback, FlushReport, LoggingWorker, ShutdownReport, WorkerSettings,
};
use serde_json::{Value, json};
use tracing::Span;

use super::{EventInput, Input, Outcome, Step};

#[derive(Debug, thiserror::Error)]
pub(super) enum TraceError {
    #[error(transparent)]
    Worker(#[from] betterlite_logging::worker::WorkerError),
    #[error("the logging worker rejected the event: {0:?}")]
    Rejected(Admission),
    #[error("the scripted steps use the logging worker after it was stopped")]
    Stopped,
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted logging callback failed")]
struct ScriptedFailure;

#[derive(Default)]
struct Record {
    completed: Mutex<Vec<String>>,
    failed: Mutex<Vec<String>>,
    contexts: Mutex<BTreeMap<String, String>>,
}

#[derive(Default)]
struct SpanContexts(Mutex<BTreeMap<u64, String>>);

impl SpanContexts {
    fn bind(&self, span: &Span, context: &str) {
        if let Some(id) = span.id() {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.into_u64(), context.to_owned());
        }
    }

    fn resolve(&self, span: &Span) -> String {
        let context = span.id().and_then(|id| {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id.into_u64())
                .cloned()
        });
        context.unwrap_or_else(|| "missing".to_owned())
    }
}

struct ScriptedCallback {
    name: String,
    outcome: Outcome,
    record: Arc<Record>,
    spans: Arc<SpanContexts>,
}

impl Callback for ScriptedCallback {
    type Error = ScriptedFailure;

    fn name(&self) -> &str {
        &self.name
    }

    async fn call(self) -> Result<(), Self::Error> {
        let context = self.spans.resolve(&Span::current());
        self.record
            .contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(self.name.clone(), context);
        tokio::task::yield_now().await;
        if matches!(self.outcome, Outcome::Failure) {
            self.record
                .failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(self.name);
            return Err(ScriptedFailure);
        }
        self.record
            .completed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.name);
        Ok(())
    }
}

pub(super) async fn scenario(input: &Input) -> Result<Value, TraceError> {
    let settings = input.settings.resolve();
    let record = Arc::new(Record::default());
    let spans = Arc::new(SpanContexts::default());
    let mut worker = Some(LoggingWorker::start(settings)?);
    let mut flushes = Vec::new();
    let shutdown = drive(&mut worker, &record, &spans, &input.steps, &mut flushes).await?;
    let Some(report) = shutdown else {
        return Err(TraceError::Stopped);
    };
    Ok(report_json(&settings, &record, &flushes, report))
}

async fn drive(
    worker: &mut Option<LoggingWorker<ScriptedCallback>>,
    record: &Arc<Record>,
    spans: &Arc<SpanContexts>,
    steps: &[Step],
    flushes: &mut Vec<Value>,
) -> Result<Option<ShutdownReport>, TraceError> {
    let mut shutdown = None;
    for step in steps {
        match step {
            Step::Enqueue { events } => {
                let Some(active) = worker.as_ref() else {
                    return Err(TraceError::Stopped);
                };
                for event in events {
                    enqueue(active, record, spans, event)?;
                }
            }
            Step::Flush => {
                let Some(active) = worker.as_ref() else {
                    return Err(TraceError::Stopped);
                };
                flushes.push(flush_json(&active.flush().await));
            }
            Step::Stop => {
                if let Some(active) = worker.take() {
                    shutdown = Some(active.stop().await);
                }
            }
        }
    }
    if let Some(active) = worker.take() {
        shutdown = Some(active.stop().await);
    }
    Ok(shutdown)
}

fn enqueue(
    worker: &LoggingWorker<ScriptedCallback>,
    record: &Arc<Record>,
    spans: &Arc<SpanContexts>,
    event: &EventInput,
) -> Result<(), TraceError> {
    let callback = ScriptedCallback {
        name: event.name.clone(),
        outcome: event.outcome,
        record: Arc::clone(record),
        spans: Arc::clone(spans),
    };
    let span = tracing::info_span!("worker_trace_request");
    spans.bind(&span, &event.context);
    let entered = span.enter();
    let admission = worker.enqueue(callback);
    drop(entered);
    match admission {
        Admission::Accepted => Ok(()),
        rejected @ Admission::Rejected(_) => Err(TraceError::Rejected(rejected)),
    }
}

fn flush_json(report: &FlushReport) -> Value {
    let pending = report
        .accepted
        .saturating_sub(report.completed)
        .saturating_sub(report.failed)
        .saturating_sub(report.timed_out);
    json!({
        "completed": report.completed,
        "failed": report.failed,
        "pending": pending,
    })
}

fn report_json(
    settings: &WorkerSettings,
    record: &Record,
    flushes: &[Value],
    report: ShutdownReport,
) -> Value {
    let mut completed = record
        .completed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    completed.sort();
    let mut failed = record
        .failed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    failed.sort();
    let contexts = record
        .contexts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let status = if report.drained {
        "stopped"
    } else {
        "cancelled"
    };
    json!({
        "settings": {
            "max_queue_size": settings.max_queue_size,
            "concurrency": settings.concurrency,
            "timeout_milliseconds": u64::try_from(settings.event_timeout.as_millis()).unwrap_or(u64::MAX),
            "timeout_summary_window_milliseconds": u64::try_from(settings.timeout_summary_window.as_millis()).unwrap_or(u64::MAX),
        },
        "accepted": report.accepted,
        "completed": completed,
        "failed": failed,
        "contexts": contexts,
        "flushes": flushes,
        "status": status,
    })
}
