use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Clone, Default)]
pub(super) struct Watch {
    lines: Arc<Mutex<Vec<Line>>>,
}

pub(super) struct Line {
    level: &'static str,
    message: String,
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Watch {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARNING",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        if level != "WARNING" && level != "DEBUG" {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Line {
                level,
                message: message.0,
            });
    }
}

impl Watch {
    pub(super) fn line_count(&self) -> usize {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn lines_since(&self, start: usize) -> Value {
        let lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        Value::Array(
            lines
                .iter()
                .skip(start)
                .map(|line| json!({"level": line.level, "message": line.message}))
                .collect(),
        )
    }
}

pub(super) fn install() -> (DefaultGuard, Watch) {
    let watch = Watch::default();
    let subscriber = tracing_subscriber::registry().with(watch.clone());
    (set_default(subscriber), watch)
}
