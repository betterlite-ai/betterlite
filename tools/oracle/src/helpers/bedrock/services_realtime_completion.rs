use betterlite_bedrock::realtime_completion::content_end::content_end_events;
use betterlite_bedrock::realtime_completion::records::{
    CompletionEvent, DeltaChunk, DeltaType, ResponseState,
};
use betterlite_bedrock::realtime_completion::tool_input::ToolUseEvent;
use betterlite_bedrock::realtime_completion::tool_use::tool_use_events;
use betterlite_bedrock::realtime_completion::usage::{
    leftover_usage_done_events, response_done_events,
};
use betterlite_bedrock::realtime_session::RealtimeSession;
use betterlite_bedrock::realtime_session::records::UsageEvent;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::services_realtime_session::FixedIds;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    ids: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Step {
    SetState(StateInput),
    // Value keeps the counts out of serde's tag buffer, which breaks arbitrary-precision numbers
    Usage { usage: Value },
    ContentEnd,
    PromptEnd,
    TakeUsage,
    Leftover,
    ToolUse { event: Value },
}

#[derive(Debug, Deserialize)]
struct StateInput {
    output_item_id: Option<String>,
    response_id: Option<String>,
    conversation_id: Option<String>,
    delta_type: Option<String>,
    delta_chunks: Option<Vec<String>>,
}

impl From<&StateInput> for ResponseState {
    fn from(input: &StateInput) -> Self {
        // An unknown delta type behaves like none: only text and audio get done events
        let delta_type = match input.delta_type.as_deref() {
            Some("text") => Some(DeltaType::Text),
            Some("audio") => Some(DeltaType::Audio),
            Some(_) | None => None,
        };
        Self {
            output_item_id: input.output_item_id.clone(),
            response_id: input.response_id.clone(),
            conversation_id: input.conversation_id.clone(),
            delta_type,
            delta_chunks: input.delta_chunks.as_ref().map(|texts| {
                texts
                    .iter()
                    .map(|delta| DeltaChunk {
                        delta: delta.clone(),
                    })
                    .collect()
            }),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let uuids = input
            .ids
            .iter()
            .map(|text| Uuid::parse_str(text))
            .collect::<Result<Vec<_>, _>>()
            .or_value_error()?;
        let ids = FixedIds::new(&uuids);
        let mut session = RealtimeSession::new(&FixedIds::new(&[]));
        let mut state = ResponseState::default();
        let mut results = Vec::new();
        for step in &input.steps {
            results.push(replay(step, &mut session, &mut state, &ids)?);
        }
        Ok(json!({ "results": results }))
    }
}

fn replay(
    step: &Step,
    session: &mut RealtimeSession,
    state: &mut ResponseState,
    ids: &FixedIds,
) -> HelperResult {
    match step {
        Step::SetState(input) => {
            *state = ResponseState::from(input);
            Ok(Value::Null)
        }
        Step::Usage { usage } => {
            session.cumulative_usage = serde_json::from_value::<UsageEvent>(usage.clone())?;
            Ok(Value::Null)
        }
        Step::ContentEnd => {
            let events = content_end_events(state, ids);
            let chunks = state.delta_chunks.as_ref().map(|kept| {
                kept.iter()
                    .map(|chunk| chunk.delta.as_str())
                    .collect::<Vec<_>>()
            });
            Ok(json!({ "events": serde_json::to_value(&events)?, "delta_chunks": chunks }))
        }
        Step::PromptEnd => prompt_end(session, state, ids),
        Step::TakeUsage => take_usage(session, ids),
        Step::Leftover => {
            let events = leftover_usage_done_events(session, ids).or_value_error()?;
            Ok(json!({ "events": serde_json::to_value(&events)? }))
        }
        Step::ToolUse { event } => tool_use(state, event, ids),
    }
}

fn prompt_end(
    session: &mut RealtimeSession,
    state: &mut ResponseState,
    ids: &FixedIds,
) -> HelperResult {
    let events = response_done_events(session, state, ids).or_value_error()?;
    Ok(json!({
        "events": serde_json::to_value(&events)?,
        "output_item_id": state.output_item_id,
        "response_id": state.response_id,
        "delta_type": Value::Null,
    }))
}

fn tool_use(state: &ResponseState, event: &Value, ids: &FixedIds) -> HelperResult {
    let parsed: ToolUseEvent = serde_json::from_str(&event.to_string())?;
    let output = tool_use_events(state, &parsed, ids).or_value_error()?;
    let mut events = serde_json::to_value(&output.events)?;
    // json.dumps spacing is not part of the contract, so both sides compare the decoded arguments
    if let Some(CompletionEvent::FunctionCallArgumentsDone(done)) = output.events.first()
        && let Some(slot) = events.pointer_mut("/0/arguments")
    {
        *slot = serde_json::from_str(&done.arguments)?;
    }
    Ok(json!({
        "events": events,
        "tool_call_id": output.tool_call_id,
        "tool_name": output.tool_name,
    }))
}

// LiteLLM's usage delta is private, so both sides reach it through the public response.done
fn take_usage(session: &mut RealtimeSession, ids: &FixedIds) -> HelperResult {
    let mut scratch = ResponseState {
        output_item_id: None,
        response_id: Some("resp-take".to_owned()),
        conversation_id: Some("conv-take".to_owned()),
        delta_type: None,
        delta_chunks: None,
    };
    let events = response_done_events(session, &mut scratch, ids).or_value_error()?;
    match events.first() {
        Some(CompletionEvent::ResponseDone(done)) => {
            Ok(serde_json::to_value(&done.response.usage)?)
        }
        Some(_) | None => Ok(Value::Null),
    }
}
