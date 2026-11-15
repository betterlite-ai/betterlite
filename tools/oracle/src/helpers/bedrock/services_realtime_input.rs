use betterlite_bedrock::realtime_input::RealtimeInput;
use betterlite_bedrock::realtime_input::decode::{ClientEvent, decode_realtime_client_input};
use betterlite_bedrock::realtime_input::error::InputError;
use betterlite_bedrock::realtime_session::RealtimeSession;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::services_realtime_session::FixedIds;
use crate::helpers::{Helper, HelperResult, OrHelperError};

mod tests;

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
            .map(|id| Uuid::parse_str(id))
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
            let warnings = warning_kinds(message);
            match state.transform_realtime_request(message.as_bytes(), source) {
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

fn warning_kinds(message: &str) -> Vec<String> {
    match decode_realtime_client_input(message.as_bytes()) {
        Err(InputError::InvalidJson { .. }) => vec!["invalid_json".to_owned()],
        Ok(ClientEvent::Unknown(name)) => vec![format!("unknown_type:{name}")],
        Ok(_) | Err(InputError::Malformed { .. }) => Vec::new(),
    }
}
