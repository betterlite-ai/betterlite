use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Clone, Default)]
pub(super) struct Watch {
    notes: Arc<Mutex<Notes>>,
}

#[derive(Default)]
struct Notes {
    warnings: Vec<String>,
    transformation: bool,
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

impl Watch {
    pub(super) fn take(&self) -> (Vec<String>, bool) {
        match self.notes.lock() {
            Ok(notes) => (notes.warnings.clone(), notes.transformation),
            Err(poisoned) => {
                let notes = poisoned.into_inner();
                (notes.warnings.clone(), notes.transformation)
            }
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Watch {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        let mut line = Line::default();
        event.record(&mut line);
        let mut notes = match self.notes.lock() {
            Ok(notes) => notes,
            Err(poisoned) => poisoned.into_inner(),
        };
        if level == Level::WARN {
            notes.warnings.push(line.0);
        } else if level == Level::DEBUG && line.0.contains("TRANSFORMATION DEBUG") {
            notes.transformation = true;
        }
    }
}

pub(super) fn install() -> (DefaultGuard, Watch) {
    let watch = Watch::default();
    let subscriber = tracing_subscriber::registry().with(watch.clone());
    (set_default(subscriber), watch)
}
