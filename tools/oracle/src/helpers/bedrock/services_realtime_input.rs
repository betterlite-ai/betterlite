use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use betterlite_bedrock::realtime_input::RealtimeInput;
use betterlite_bedrock::realtime_input::error::InputError;
use betterlite_bedrock::realtime_session::RealtimeSession;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use uuid::Uuid;

use super::services_realtime_session::FixedIds;
use crate::helpers::{Helper, HelperResult, OrHelperError};

mod tests;

const INVALID_JSON: &str = "Invalid JSON message";
const UNKNOWN_TYPE: &str = "Unknown message type: ";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    ids: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Send { message: String },
    Close,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let ids = input
            .ids
            .iter()
            .map(String::as_str)
            .map(Uuid::parse_str)
            .collect::<Result<Vec<Uuid>, _>>()
            .or_value_error()?;
        let source = FixedIds::new(&ids);
        let mut state = RealtimeInput {
            session: RealtimeSession::new(&source),
            open_audio: None,
        };
        let mut steps: Vec<Value> = Vec::new();
        for step in &input.steps {
            steps.push(run_step(&mut state, step, &source)?);
        }
        Ok(json!({ "steps": steps }))
    }
}

fn run_step(state: &mut RealtimeInput, step: &Step, source: &FixedIds) -> HelperResult {
    let (ok, frames, warnings) = match step {
        Step::Close => (
            true,
            serde_json::to_value(state.session_close_messages())?,
            Vec::new(),
        ),
        Step::Send { message } => {
            let warnings = Warnings::default();
            let subscriber = registry().with(warnings.clone());
            let outcome = with_default(subscriber, || {
                state.transform_realtime_request(message.as_bytes(), source)
            });
            let captured = warnings
                .0
                .lock()
                .map(|messages| messages.clone())
                .unwrap_or_default();
            let warnings = warning_kinds(&captured);
            match outcome {
                Ok(frames) => (true, serde_json::to_value(frames)?, warnings),
                Err(InputError::Malformed { .. } | InputError::InvalidJson { .. }) => {
                    (false, json!([]), warnings)
                }
            }
        }
    };
    Ok(json!({
        "ok": ok,
        "frames": frames,
        "warnings": warnings,
        "state": {
            "prompt_started": state.session.prompt_started,
            "client_audio_streamed": state.session.client_audio_streamed,
            "audio_content_name": state.session.names.audio_content,
        },
    }))
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        if let Ok(mut messages) = self.0.lock() {
            messages.push(message.0.trim_matches('"').to_owned());
        }
    }
}

fn warning_kinds(messages: &[String]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| {
            if message.starts_with(INVALID_JSON) {
                Some("invalid_json".to_owned())
            } else {
                message
                    .strip_prefix(UNKNOWN_TYPE)
                    .map(|kind| format!("unknown_type:{kind}"))
            }
        })
        .collect()
}
