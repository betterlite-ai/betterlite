use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

const PREFIX: &str = "Policy engine: ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct LogEntry {
    pub level: &'static str,
    pub message: String,
}

pub(super) fn captured<T, F>(run: F) -> (T, Vec<LogEntry>)
where
    F: FnOnce() -> T,
{
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let result = with_default(subscriber, run);
    let entries = capture
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (result, entries)
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<LogEntry>>>);

impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let Some(name) = level_name(*event.metadata().level()) else {
            return;
        };
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        if !visitor.0.starts_with(PREFIX) {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LogEntry {
                level: name,
                message: visitor.0,
            });
    }
}

fn level_name(level: Level) -> Option<&'static str> {
    if level == Level::WARN {
        return Some("warning");
    }
    if level == Level::DEBUG {
        return Some("debug");
    }
    None
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
