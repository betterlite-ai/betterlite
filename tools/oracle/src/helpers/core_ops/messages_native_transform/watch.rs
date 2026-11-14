use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Clone, Default)]
pub(super) struct Watch {
    pub(super) notes: Arc<Mutex<Notes>>,
}

#[derive(Default)]
pub(super) struct Notes {
    pub(super) warnings: Vec<String>,
    pub(super) transformation: bool,
}

#[derive(Default)]
struct Line(String);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Watch {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        let mut line = Line::default();
        event.record(&mut line);
        let mut notes = self.notes.lock().unwrap_or_else(PoisonError::into_inner);
        if level == Level::WARN {
            notes.warnings.push(line.0);
            return;
        }
        if level == Level::DEBUG && line.0.contains("TRANSFORMATION DEBUG") {
            notes.transformation = true;
        }
    }
}

pub(super) fn install() -> (DefaultGuard, Watch) {
    let watch = Watch::default();
    let subscriber = tracing_subscriber::registry().with(watch.clone());
    (set_default(subscriber), watch)
}
