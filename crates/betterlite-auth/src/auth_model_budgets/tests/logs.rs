use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Clone, Debug)]
pub(crate) struct Logged {
    pub(crate) level: Level,
    pub(crate) message: String,
    pub(crate) fields: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
pub(crate) struct Logs {
    entries: Arc<Mutex<Vec<Logged>>>,
}

impl Logs {
    pub(crate) fn install(&self) -> DefaultGuard {
        let capture = Capture {
            entries: Arc::clone(&self.entries),
        };
        set_default(tracing_subscriber::registry().with(capture))
    }

    pub(crate) fn matching(&self, level: Level, text: &str) -> Vec<Logged> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.level == level && entry.message.contains(text))
            .cloned()
            .collect()
    }

    pub(crate) fn debugs(&self, text: &str) -> Vec<Logged> {
        self.matching(Level::DEBUG, text)
    }
}

struct Capture {
    entries: Arc<Mutex<Vec<Logged>>>,
}

impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut visitor = Captured::default();
        event.record(&mut visitor);
        let message = visitor.fields.get("message").cloned().unwrap_or_default();
        self.entries.lock().unwrap().push(Logged {
            level: *event.metadata().level(),
            message,
            fields: visitor.fields,
        });
    }
}

#[derive(Default)]
struct Captured {
    fields: BTreeMap<String, String>,
}

impl Visit for Captured {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }
}
