use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

#[derive(Debug, Clone, Serialize)]
pub(super) struct LogLine {
    level: &'static str,
    message: String,
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<LogLine>>>);

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

// only the compaction diagnostics are compared; the converters underneath log on their own
impl<Inner: Subscriber> Layer<Inner> for Recorder {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut field = MessageField::default();
        event.record(&mut field);
        if !field.0.starts_with("compact_20260112:") {
            return;
        }
        let level = *event.metadata().level();
        let name = if level == Level::WARN {
            "warning"
        } else if level == Level::INFO {
            "info"
        } else {
            "debug"
        };
        let line = LogLine {
            level: name,
            message: field.0,
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }
}

pub(super) fn captured<Output, Call>(call: Call) -> (Output, Vec<LogLine>)
where
    Call: FnOnce() -> Output,
{
    let recorder = Recorder::default();
    let value = with_default(registry().with(recorder.clone()), call);
    let lines = recorder
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, lines)
}
