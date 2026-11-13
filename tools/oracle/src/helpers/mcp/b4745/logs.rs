use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::Subscriber;
use tracing::{Event, Level};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const IGNORED: [&str; 2] = [
    "run_with_session failed",
    "waiting for a valid MCP response",
];

const TAGS: [(&str, &str); 18] = [
    ("MCP client listing tools from", "listing"),
    ("MCP client listed", "listed"),
    ("list_tools was cancelled", "list_cancelled"),
    ("list_tools failed", "list_failed"),
    ("during list_tools", "list_broken"),
    ("MCP client calling tool", "calling"),
    ("progress:", "progress"),
    ("Failed to forward to Host", "host_failed"),
    ("sending tool call to session", "sending"),
    ("completed successfully", "call_completed"),
    ("tool call timed out after", "call_cancelled"),
    ("call_tool failed", "call_failed"),
    ("tool call traceback", "traceback"),
    ("detected broken connection/stream -", "call_broken"),
    ("repeated a tools/list cursor", "cursor_repeat"),
    ("pagination exceeded the maximum", "page_limit"),
    ("reached the 1,000 page limit", "page_limit"),
    ("listing deadline", "deadline"),
];

fn tag(message: &str) -> Option<&'static str> {
    if IGNORED.iter().any(|needle| message.contains(needle)) {
        return None;
    }
    TAGS.iter()
        .find_map(|(needle, name)| message.contains(needle).then_some(*name))
}

#[derive(Clone, Default)]
pub(crate) struct Logs {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

impl Logs {
    pub(crate) fn entries(&self) -> Vec<(String, String)> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn reset(&self) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    fn record(&self, level: Level, message: &str) {
        let Some(name) = tag(message) else {
            return;
        };
        let name_level = match level {
            Level::DEBUG => "debug",
            Level::INFO => "info",
            Level::WARN => "warn",
            Level::ERROR => "error",
            Level::TRACE => return,
        };
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((name_level.to_owned(), name.to_owned()));
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Current> Layer<Current> for Logs
where
    Current: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Current>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        self.record(*event.metadata().level(), &message.0);
    }
}
