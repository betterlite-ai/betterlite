use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;
use betterlite_vertex::gemini_live_content::delta::content_delta_event;
use betterlite_vertex::gemini_live_content::ids::ContentIds;
use betterlite_vertex::gemini_live_content::map::{ModalityEvent, map_openai_event};
use betterlite_vertex::gemini_live_content::modality::handle_openai_modality_event;
use betterlite_vertex::gemini_live_content::start::{StartContent, start_content_events};
use betterlite_vertex::gemini_live_content::state::ContentState;
use betterlite_vertex::gemini_live_content::update::{
    ChunkKind, ChunkSource, update_current_chunks,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::warnings::Warnings;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    MapEvent(MapEventCall),
    Start(StartCall),
    Delta(DeltaCall),
    Modality(ModalityCall),
    Update(UpdateCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapEventCall {
    key: String,
    value: Json,
    #[serde(default)]
    delta_type: Option<AllDeltaType>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartCall {
    response_id: String,
    output_item_id: String,
    conversation_id: String,
    delta_type: AllDeltaType,
    #[serde(default)]
    session_configuration_request: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeltaCall {
    server_content: Json,
    output_item_id: String,
    response_id: String,
    delta_type: AllDeltaType,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModalityCall {
    event: ModalityEvent,
    delta_type: AllDeltaType,
    #[serde(default)]
    frame: Option<Json>,
    #[serde(default)]
    session_configuration_request: Option<String>,
    #[serde(default)]
    output_item_id: Option<String>,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    delta_chunks: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateCall {
    chunks: ChunkKind,
    steps: Vec<UpdateStep>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum UpdateStep {
    Single { event: Json },
    List { events: Vec<Json> },
}

#[derive(Default)]
struct CounterIds {
    next: u64,
}

impl ContentIds for CounterIds {
    fn next_id(&mut self) -> String {
        let value = format!("00000000-0000-4000-8000-{:012x}", self.next);
        self.next = self.next.saturating_add(1);
        value
    }
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let value = with_default(subscriber, || dispatch(call))?;
        answers.push(json!({"value": value, "warnings": warnings.collected()}));
    }
    Ok(json!({"calls": answers}))
}

fn dispatch(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::MapEvent(call) => Ok(json!({
            "event": map_openai_event(&call.key, &call.value, call.delta_type).or_value_error()?,
        })),
        Call::Start(call) => Ok(json!({
            "events": start_content_events(
                &StartContent {
                    setup_request: call.session_configuration_request.as_deref(),
                    response_id: call.response_id.as_str(),
                    output_item_id: call.output_item_id.as_str(),
                    conversation_id: call.conversation_id.as_str(),
                    delta_type: call.delta_type,
                },
                &mut CounterIds::default(),
            )
            .or_value_error()?,
        })),
        Call::Delta(call) => Ok(json!({
            "event": content_delta_event(
                &call.server_content,
                call.output_item_id.as_str(),
                call.response_id.as_str(),
                call.delta_type,
                &mut CounterIds::default(),
            )
            .or_value_error()?,
        })),
        Call::Modality(call) => modality(call),
        Call::Update(call) => Ok(json!({
            "steps": step_states(call),
        })),
    }
}

fn modality(call: &ModalityCall) -> Result<Value, Box<HelperError>> {
    let empty = Json::Null;
    let frame = call.frame.as_ref().unwrap_or(&empty);
    let mut current = ContentState {
        output_item_id: call.output_item_id.clone(),
        response_id: call.response_id.clone(),
        conversation_id: call.conversation_id.clone(),
        delta_chunks: call.delta_chunks.clone(),
        item_chunks: None,
        delta_type: Some(call.delta_type),
    };
    let mut ids = CounterIds::default();
    let events = handle_openai_modality_event(
        &mut current,
        call.event,
        frame,
        call.session_configuration_request.as_deref(),
        &mut ids,
    )
    .or_value_error()?;
    Ok(json!({
        "returned_message": events,
        "current_output_item_id": current.output_item_id,
        "current_response_id": current.response_id,
        "current_conversation_id": current.conversation_id,
        "current_delta_chunks": current.delta_chunks,
        "current_delta_type": current.delta_type,
    }))
}

fn step_states(call: &UpdateCall) -> Vec<Option<Vec<Json>>> {
    let mut current: Option<Vec<Json>> = None;
    let mut states = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        current = match step {
            UpdateStep::Single { event } => {
                update_current_chunks(ChunkSource::Single(event), current, call.chunks)
            }
            UpdateStep::List { events } => {
                update_current_chunks(ChunkSource::List(events.as_slice()), current, call.chunks)
            }
        };
        states.push(current.clone());
    }
    states
}
