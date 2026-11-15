use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::helper_error;
use crate::helpers::HelperError;

pub(super) type Levels = Arc<Mutex<Vec<Value>>>;

struct LevelCapture {
    levels: Levels,
}

impl<S: Subscriber> Layer<S> for LevelCapture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let level = *event.metadata().level();
        if level > Level::INFO {
            return;
        }
        let name = if level == Level::ERROR {
            "error"
        } else if level == Level::WARN {
            "warn"
        } else {
            "info"
        };
        self.levels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Value::String(name.to_owned()));
    }
}

pub(super) fn capture_levels() -> (Levels, DefaultGuard) {
    let levels: Levels = Arc::new(Mutex::new(Vec::new()));
    let layer = LevelCapture {
        levels: Arc::clone(&levels),
    };
    (levels, set_default(registry().with(layer)))
}

pub(super) fn outcome(failed: bool, writes: &[Value], levels: &Levels) -> Value {
    let logs = levels
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!({"failed": failed, "writes": writes, "logs": logs})
}

pub(super) fn block_on<F: Future>(future: F) -> Result<F::Output, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|source| helper_error("RuntimeError", &source.to_string()))?;
    Ok(runtime.block_on(future))
}
