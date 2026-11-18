use std::fmt::Display;

use betterlite_bedrock::realtime_events::dispatch::RealtimeEvents;
use betterlite_bedrock::realtime_events::error::EventError;
use betterlite_bedrock::realtime_session::RealtimeSession;
use betterlite_bedrock::realtime_session::records::{SessionEvent, SessionEventType};
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::services_realtime_session::FixedIds;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

const MODEL: &str = "amazon.nova-sonic-v1:0";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    ids: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
enum Step {
    Dispatch {
        message: String,
        state: RealtimeResponseTransformInput,
    },
    SessionUpdated {
        trace_id: String,
        modalities: Option<Vec<String>>,
    },
}

#[derive(Serialize)]
struct Steps {
    steps: Vec<StepOutput>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum StepOutput {
    Dispatch {
        response: Option<RealtimeResponseTypedDict>,
        raised: Option<&'static str>,
    },
    SessionUpdated {
        event: SessionEvent,
    },
}

pub(crate) struct Op;

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RealtimeEvents".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn parse_ids(values: &[String]) -> Result<Vec<Uuid>, Box<HelperError>> {
    values
        .iter()
        .map(String::as_str)
        .map(Uuid::parse_str)
        .collect::<Result<Vec<Uuid>, _>>()
        .map_err(failure)
}

// A declared-type message only fails on the shapes Python answers with its own exception class
const fn raised(error: &EventError) -> &'static str {
    match error {
        EventError::Utf8 { .. } => "invalid_utf8",
        EventError::NotAnObject { .. } => "not_an_object",
        EventError::EventNotAnObject { .. } => "event_not_an_object",
        EventError::Payload { .. } => "invalid_payload",
        EventError::Encode { .. } => "encode",
        EventError::Completion(_) => "completion",
    }
}

fn dispatch_step(
    message: &str,
    state: &RealtimeResponseTransformInput,
    session: &mut RealtimeSession,
    ids: &FixedIds,
    redactor: &Redactor,
) -> StepOutput {
    let mut events = RealtimeEvents {
        session: &mut *session,
        ids,
        redactor,
    };
    match events.transform_realtime_response(message.as_bytes(), state) {
        Ok(response) => StepOutput::Dispatch {
            response: Some(response),
            raised: None,
        },
        Err(error) => StepOutput::Dispatch {
            response: None,
            raised: Some(raised(&error)),
        },
    }
}

fn run_step(
    step: &Step,
    session: &mut RealtimeSession,
    ids: &FixedIds,
    redactor: &Redactor,
) -> StepOutput {
    match step {
        Step::Dispatch { message, state } => dispatch_step(message, state, session, ids, redactor),
        Step::SessionUpdated {
            trace_id,
            modalities,
        } => StepOutput::SessionUpdated {
            event: RealtimeSession::session_event(
                SessionEventType::Updated,
                Some(MODEL),
                trace_id,
                modalities.as_deref(),
                ids,
            ),
        },
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let ids = parse_ids(&input.ids)?;
        let source = FixedIds::new(&ids);
        let mut session = RealtimeSession::new(&FixedIds::new(&[]));
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
        let steps = input
            .steps
            .iter()
            .map(|step| run_step(step, &mut session, &source, &redactor))
            .collect();
        serde_json::to_value(Steps { steps }).map_err(failure)
    }
}
