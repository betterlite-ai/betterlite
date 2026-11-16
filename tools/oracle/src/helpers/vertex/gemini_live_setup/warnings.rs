use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

const STOCK_VOICE: &str = "is an OpenAI voice with no Gemini equivalent";
const DROPPED_UPDATE: &str = "rejects a second setup message";

#[derive(Clone, Default)]
pub(super) struct Warnings {
    messages: Arc<Mutex<Vec<String>>>,
}

impl Warnings {
    pub(super) fn labels(&self) -> Vec<String> {
        self.messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|message| label(message))
            .collect()
    }
}

fn label(message: &str) -> String {
    if message.contains(STOCK_VOICE) {
        "stock_voice_dropped".to_owned()
    } else if message.contains(DROPPED_UPDATE) {
        "later_update_dropped_with_guardrail".to_owned()
    } else {
        message.to_owned()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut field = MessageField::default();
        event.record(&mut field);
        self.messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(field.0);
    }
}
