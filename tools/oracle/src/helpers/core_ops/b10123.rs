use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

// ast-grep-ignore: no-long-names
use betterlite_core::api::chat_mcp_followup::create_follow_up_messages_for_chat;
use betterlite_core::api::responses_mcp_results::McpToolResult;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber, subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    original_messages: Vec<Json>,
    body: ChatResponse,
    tool_results: Vec<McpToolResult>,
}

#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<Value>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Lines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARNING",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": level, "message": message.0}));
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let lines = Lines::default();
        let guard = subscriber::set_default(tracing_subscriber::registry().with(lines.clone()));
        // ast-grep-ignore: no-long-names
        let outcome = create_follow_up_messages_for_chat(
            &input.original_messages,
            &input.body,
            &input.tool_results,
        );
        drop(guard);
        let messages = outcome.map_err(|error| failure("ChatFollowupError", error.to_string()))?;
        let records = lines
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({"messages": messages, "records": records}))
    }
}

mod tests;
