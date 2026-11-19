use std::fmt::Debug;

use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use super::{Shared, lock};

struct Capture(String);

impl Visit for Capture {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

pub(crate) struct Warnings(pub(crate) Shared);

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if !event
            .metadata()
            .target()
            .starts_with("betterlite_auth::auth_pubsub")
            || !matches!(level, Level::WARN | Level::ERROR)
        {
            return;
        }
        let mut capture = Capture(String::new());
        event.record(&mut capture);
        let name = if level == Level::WARN {
            "warning"
        } else {
            "error"
        };
        lock(&self.0)
            .warnings
            .push(json!({ "level": name, "message": capture.0 }));
    }
}
