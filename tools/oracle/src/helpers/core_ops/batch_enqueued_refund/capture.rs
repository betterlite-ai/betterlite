use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

pub(super) struct CaptureLayer(pub(super) Arc<Mutex<Vec<Value>>>);

impl<Inner: Subscriber> Layer<Inner> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        // the source reports every refund diagnostic through verbose_proxy_logger at WARNING
        if *event.metadata().level() != Level::WARN {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"event": "warning", "level": "warning"}));
    }
}
