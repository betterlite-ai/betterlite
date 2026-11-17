use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Clone, Default)]
pub(crate) struct Capture {
    events: Arc<Mutex<Vec<Line>>>,
}

#[derive(Clone, Debug)]
struct Line {
    level: Level,
    message: String,
}

impl Capture {
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.at(Level::WARN)
    }

    pub(crate) fn infos(&self) -> Vec<String> {
        self.at(Level::INFO)
    }

    pub(crate) fn errors(&self) -> Vec<String> {
        self.at(Level::ERROR)
    }

    fn at(&self, level: Level) -> Vec<String> {
        let events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        events
            .iter()
            .filter(|line| line.level == level)
            .map(|line| line.message.clone())
            .collect()
    }
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

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        events.push(Line {
            level: *event.metadata().level(),
            message: message.0,
        });
    }
}

pub(crate) fn install() -> (DefaultGuard, Capture) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    (set_default(subscriber), capture)
}
