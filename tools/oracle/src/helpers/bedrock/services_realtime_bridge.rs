use std::fmt::Display;

use betterlite_bedrock::realtime_input::RealtimeInput;
use betterlite_bedrock::realtime_session::RealtimeSession;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::services_realtime_session::FixedIds;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod handlers;
mod tests;
mod transports;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    // Three names per run: the config names are consumed when a session is built, before any event id
    name_ids: Vec<String>,
    event_ids: Vec<String>,
    runs: Vec<Run>,
}

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
struct CaseFlag(bool);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Run {
    kind: Kind,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    initial: Option<String>,
    #[serde(default)]
    frames: Vec<Option<String>>,
    #[serde(default)]
    chunks: Vec<Chunk>,
    #[serde(default)]
    await_error: Option<String>,
    #[serde(default)]
    block: CaseFlag,
    #[serde(default)]
    cancel: CaseFlag,
    #[serde(default)]
    bedrock_send_failures: Vec<usize>,
    #[serde(default)]
    receive_failures: Vec<usize>,
    #[serde(default)]
    client_send_failures: Vec<usize>,
    #[serde(default)]
    bedrock_close_error: CaseFlag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Client,
    Provider,
}

// One provider output chunk; kind names which SDK value the fake stream hands the bridge
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    kind: ChunkKind,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChunkKind {
    Bytes,
    InvalidUtf8,
    MissingValue,
    MissingBytes,
    Empty,
    End,
}

#[derive(Debug, Serialize)]
struct Runs {
    runs: Vec<BridgeView>,
}

#[derive(Debug, Default, Serialize)]
struct BridgeView {
    outcome: &'static str,
    error: Option<&'static str>,
    sent_to_bedrock: Vec<Json>,
    sent_to_client: Vec<Json>,
    closed_input: bool,
    closed_client: Option<ClosedClient>,
    scope: Option<ScopeView>,
    state: Option<RealtimeResponseTransformInput>,
    logged: Vec<Json>,
}

#[derive(Debug, Serialize)]
struct ClosedClient {
    code: u16,
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct ScopeView {
    pending_session_update: Option<String>,
    session_committed: bool,
}

pub(crate) struct Op;

pub(super) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RealtimeBridge".to_owned(),
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

const fn empty_state() -> RealtimeResponseTransformInput {
    RealtimeResponseTransformInput {
        session_configuration_request: None,
        current_output_item_id: None,
        current_response_id: None,
        current_delta_chunks: None,
        current_item_chunks: None,
        current_conversation_id: None,
        current_delta_type: None,
    }
}

// Session names and event ids come from two generators in LiteLLM, so each run seeds names and events apart
fn session_input(name_ids: &[Uuid]) -> RealtimeInput {
    RealtimeInput {
        session: RealtimeSession::new(&FixedIds::new(name_ids)),
        open_audio: None,
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let names = parse_ids(&input.name_ids)?;
        let events = parse_ids(&input.event_ids)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
        handlers::runs(input, &names, &FixedIds::new(&events), &redactor)
    }
}
