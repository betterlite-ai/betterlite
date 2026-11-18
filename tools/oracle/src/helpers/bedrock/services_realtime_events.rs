use std::fmt::Display;

use betterlite_bedrock::realtime_events::decode::decode_realtime_client_preface;
use betterlite_bedrock::realtime_events::dispatch::RealtimeEvents;
use betterlite_bedrock::realtime_events::error::EventError;
use betterlite_bedrock::realtime_events::logging::should_log_event;
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_bedrock::realtime_session::RealtimeSession;
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::services_realtime_session::FixedIds;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod handlers;
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
    ClientMessages {
        initial: Option<String>,
        received: Vec<Option<String>>,
    },
    ParseClientMessage {
        message: String,
    },
    Pending {
        scope: EventScope,
    },
    ProviderFailure {
        class_name: String,
        message: String,
        scope: EventScope,
    },
    Ack(AckStep),
    ShouldLog {
        configured: Option<ConfiguredEventTypes>,
        event: IndexMap<String, Json>,
    },
}

// litellm.logged_real_time_event_types is "*", a list of event names or None
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ConfiguredEventTypes {
    All(AllEventTypes),
    Only(Vec<String>),
}

#[derive(Debug, Deserialize)]
enum AllEventTypes {
    #[serde(rename = "*")]
    Star,
}

#[derive(Debug, Deserialize)]
struct AckStep {
    trace_id: Option<String>,
    client_message: IndexMap<String, Json>,
    stream_error: Option<String>,
    send_error: Option<String>,
    scope: EventScope,
}

#[derive(Serialize)]
struct Steps {
    steps: Vec<StepOutput>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum StepOutput {
    Dispatch(DispatchOutput),
    ClientMessages(ClientMessagesOutput),
    ParseClientMessage(ParseClientMessageOutput),
    Pending(PendingOutput),
    Provider(ProviderOutput),
    Ack(AckOutput),
    ShouldLog(ShouldLogOutput),
}

#[derive(Serialize)]
struct DispatchOutput {
    response: Option<RealtimeResponseTypedDict>,
    raised: Option<&'static str>,
}

#[derive(Serialize)]
struct ClientMessagesOutput {
    messages: Vec<String>,
    receive_calls: u32,
    after_end: Option<String>,
}

#[derive(Serialize)]
struct ParseClientMessageOutput {
    parsed: IndexMap<String, Json>,
}

#[derive(Serialize)]
struct PendingOutput {
    pending: Option<String>,
    status: Option<u16>,
}

#[derive(Serialize)]
struct ProviderOutput {
    status: Option<u16>,
    recorded: bool,
    unchanged: bool,
}

#[derive(Serialize)]
struct AckOutput {
    acked: Option<bool>,
    raised: Option<&'static str>,
    scope: EventScope,
    sent: Option<SentSessionEvent>,
}

// The session.updated payload the bridge sends back to the client
#[derive(Debug, Deserialize, Serialize)]
struct SentSessionEvent {
    event_id: String,
    session: SentSession,
    #[serde(rename = "type")]
    event_type: SentEventType,
}

#[derive(Debug, Deserialize, Serialize)]
struct SentSession {
    id: String,
    modalities: Vec<String>,
    model: String,
}

#[derive(Debug, Deserialize, Serialize)]
enum SentEventType {
    #[serde(rename = "session.updated")]
    Updated,
}

#[derive(Serialize)]
struct ShouldLogOutput {
    logged: bool,
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
) -> DispatchOutput {
    let mut events = RealtimeEvents {
        session: &mut *session,
        ids,
        redactor,
    };
    match events.transform_realtime_response(message.as_bytes(), state) {
        Ok(response) => DispatchOutput {
            response: Some(response),
            raised: None,
        },
        Err(error) => DispatchOutput {
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
) -> Result<StepOutput, Box<HelperError>> {
    match step {
        Step::Dispatch { message, state } => Ok(StepOutput::Dispatch(dispatch_step(
            message, state, session, ids, redactor,
        ))),
        Step::ClientMessages { initial, received } => {
            handlers::client_messages(initial.clone(), received.clone(), redactor)
                .map(StepOutput::ClientMessages)
        }
        Step::ParseClientMessage { message } => {
            Ok(StepOutput::ParseClientMessage(ParseClientMessageOutput {
                parsed: decode_realtime_client_preface(message.as_bytes()),
            }))
        }
        Step::Pending { scope } => Ok(StepOutput::Pending(handlers::pending(scope))),
        Step::ProviderFailure {
            class_name,
            message,
            scope,
        } => Ok(StepOutput::Provider(handlers::provider(
            class_name, message, scope, redactor,
        ))),
        Step::Ack(ack) => handlers::ack(ack, ids, redactor).map(StepOutput::Ack),
        Step::ShouldLog { configured, event } => {
            let types = handlers::logged_types(configured.as_ref());
            Ok(StepOutput::ShouldLog(ShouldLogOutput {
                logged: should_log_event(&types, event),
            }))
        }
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
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_value(Steps { steps }).map_err(failure)
    }
}
