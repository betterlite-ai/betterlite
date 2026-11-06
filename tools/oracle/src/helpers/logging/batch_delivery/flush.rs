use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::batch::delivery::{BatchCancelled, BatchDelivery};
use betterlite_logging::batch::logger::{BatchQueue, BatchSettings, FlushReport};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FlushScenario {
    #[serde(default)]
    settings: SettingsInput,
    #[serde(default, rename = "preserve_events_added_during_flush")]
    preserve_during_flush: bool,
    steps: Vec<FlushStep>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SettingsInput {
    #[serde(default)]
    batch_size: Option<usize>,
    #[serde(default)]
    flush_interval_seconds: Option<u64>,
    #[serde(default)]
    max_queue_size: Option<usize>,
}

impl SettingsInput {
    fn resolve(&self) -> BatchSettings {
        let defaults = BatchSettings::default();
        BatchSettings {
            batch_size: self.batch_size.unwrap_or(defaults.batch_size),
            flush_interval: self
                .flush_interval_seconds
                .map_or(defaults.flush_interval, Duration::from_secs),
            max_queue_size: self.max_queue_size.unwrap_or(defaults.max_queue_size),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum FlushStep {
    Enqueue {
        events: Vec<String>,
    },
    Flush {
        #[serde(default)]
        script: Vec<FlushReply>,
        #[serde(default)]
        during: Vec<String>,
    },
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum FlushReply {
    Delivered,
    Failed,
}

struct ScriptedDelivery {
    script: Mutex<VecDeque<FlushReply>>,
    attempts: Mutex<Vec<Vec<String>>>,
    delivered: Mutex<Vec<String>>,
    queue: BatchQueue<String>,
    during: Vec<String>,
}

impl BatchDelivery<String> for ScriptedDelivery {
    fn deliver(
        &self,
        batch: &[String],
    ) -> impl Future<Output = Result<Vec<String>, BatchCancelled<String>>> + Send {
        let reply = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(FlushReply::Failed);
        if reply == FlushReply::Delivered {
            self.delivered
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(batch.iter().cloned());
        }
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(batch.to_vec());
        for event in &self.during {
            let _enqueued = self.queue.enqueue(event.clone());
        }
        let remaining = batch.to_vec();
        async move {
            match reply {
                FlushReply::Delivered => Ok(Vec::new()),
                FlushReply::Failed => Ok(remaining),
            }
        }
    }
}

struct Flushed {
    entry: Value,
    delivered: Vec<String>,
    overflowed: bool,
}

async fn flush_once(
    queue: &BatchQueue<String>,
    script: &[FlushReply],
    during: &[String],
) -> Flushed {
    let delivery = ScriptedDelivery {
        script: Mutex::new(script.iter().copied().collect()),
        attempts: Mutex::new(Vec::new()),
        delivered: Mutex::new(Vec::new()),
        queue: queue.clone(),
        during: during.to_vec(),
    };
    let report = queue.flush(&delivery).await;
    let delivered = delivery
        .delivered
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let sent = delivery
        .attempts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned()
        .unwrap_or_default();
    Flushed {
        entry: json!({
            "sent": sent,
            "outcome": flush_label(&report),
            "queue": queue.pending(),
            "dropped_oldest": report.dropped_oldest,
        }),
        delivered,
        overflowed: report.dropped_oldest > 0,
    }
}

pub(super) async fn flush_outcome(scenario: &FlushScenario) -> Value {
    let settings = scenario.settings.resolve();
    let mut queue = BatchQueue::new(settings);
    queue.preserve_during_flush = scenario.preserve_during_flush;
    let mut flushes = Vec::new();
    let mut delivered = Vec::new();
    let mut warnings = 0_usize;
    for step in &scenario.steps {
        match step {
            FlushStep::Enqueue { events } => {
                for event in events {
                    let _enqueued = queue.enqueue(event.clone());
                }
            }
            FlushStep::Flush { script, during } => {
                let finished = flush_once(&queue, script, during).await;
                flushes.push(finished.entry);
                delivered.extend(finished.delivered);
                if finished.overflowed {
                    warnings = warnings.saturating_add(1);
                }
            }
        }
    }
    json!({
        "settings": {
            "batch_size": settings.batch_size,
            "flush_interval_seconds": settings.flush_interval.as_secs(),
            "max_queue_size": settings.max_queue_size,
        },
        "flushes": flushes,
        "queue": queue.pending(),
        "delivered": delivered,
        "warnings": warnings,
    })
}

const fn flush_label(report: &FlushReport) -> &'static str {
    if report.batch == 0 {
        "skipped"
    } else if report.delivered == report.batch {
        "delivered"
    } else if report.delivered == 0 {
        "failed"
    } else {
        "partial"
    }
}
