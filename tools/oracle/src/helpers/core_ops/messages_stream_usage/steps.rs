use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_core::api::messages_chat_output::error::ChatOutputError;
use betterlite_core::api::messages_chat_output::records::{ChoiceRecord, UsageRecord};
use betterlite_core::api::messages_stream_usage::blocks::{
    delta_has_content, is_blank_delta, next_compaction_event, should_start_new_content_block,
    with_refusal_stop_details,
};
use betterlite_core::api::messages_stream_usage::error::StreamStateError;
use betterlite_core::api::messages_stream_usage::state::{StreamSetup, StreamState};
use betterlite_core::api::messages_stream_usage::usage::{
    augment_message_delta_usage, handle_choiceless_chunk, initial_usage, merge_usage_into_held_stop,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Object;
use serde_json::Value;
use uuid::Uuid;

use super::Input;
use super::Step;
use super::source::{ScriptedSource, logging_state};
use super::view;

pub(super) struct Harness {
    pub(super) state: StreamState<ScriptedSource>,
    pub(super) logged_id: Option<String>,
}

impl Harness {
    pub(super) fn build(input: &Input) -> Result<Self, Box<HelperError>> {
        let mut logging: Option<LoggingState> = None;
        if input.logging {
            logging = Some(logging_state()?);
        }
        let source = ScriptedSource {
            chunks_attr: input.chunks_attr.clone(),
            messages_attr: input.messages_attr.clone(),
        };
        let setup = StreamSetup {
            model: input.model.clone(),
            tool_name_mapping: input.tool_name_mapping.clone(),
            applied_edits: input.applied_edits.clone(),
            compaction_block: input.compaction_block.clone(),
            iterations_usage: input.iterations_usage.clone(),
        };
        let state = StreamState::new(logging.as_mut(), source, setup);
        let logged_id = logging.and_then(|logged| logged.streamed_anthropic_message_id);
        Ok(Self { state, logged_id })
    }
}

// B198's seam, fixed so a generated tool id is comparable.
struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

// Each failure carries the exception the pinned wrapper raises on the same input.
fn failure(error: &StreamStateError) -> Box<HelperError> {
    let class = match error {
        StreamStateError::MissingHeldStop => "AssertionError",
        StreamStateError::MalformedEvent { .. } => "KeyError",
        StreamStateError::Content {
            source: ChatOutputError::NoChoices,
        } => "IndexError",
        StreamStateError::Content { .. } | StreamStateError::Serialization { .. } => "ValueError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn hold_stop(harness: &mut Harness, event: &Object) -> Result<Value, Box<HelperError>> {
    harness.state.holding_stop_reason_chunk = Some(event.clone());
    Ok(Value::Null)
}

fn merge_held_usage(harness: &Harness, usage: &UsageRecord) -> Result<Value, Box<HelperError>> {
    match merge_usage_into_held_stop(&harness.state, usage) {
        Ok(merged) => serde_json::to_value(merged).or_value_error(),
        Err(error) => Err(failure(&error)),
    }
}

fn choiceless(
    harness: &mut Harness,
    usage: Option<&UsageRecord>,
) -> Result<Value, Box<HelperError>> {
    match handle_choiceless_chunk(&mut harness.state, usage) {
        Ok(handled) => Ok(Value::Bool(handled)),
        Err(error) => Err(failure(&error)),
    }
}

fn augment(harness: &Harness, event: &Object) -> Result<Value, Box<HelperError>> {
    match augment_message_delta_usage(&harness.state, event.clone()) {
        Ok(chunk) => serde_json::to_value(chunk).or_value_error(),
        Err(error) => Err(failure(&error)),
    }
}

fn refusal_stop(harness: &Harness, event: &Object) -> Result<Value, Box<HelperError>> {
    match with_refusal_stop_details(&harness.state, event.clone()) {
        Ok(chunk) => serde_json::to_value(chunk).or_value_error(),
        Err(error) => Err(failure(&error)),
    }
}

fn compaction(harness: &mut Harness, calls: usize) -> Result<Value, Box<HelperError>> {
    let mut events = Vec::with_capacity(calls);
    for _ in 0..calls {
        events.push(next_compaction_event(&mut harness.state));
    }
    serde_json::to_value(events).or_value_error()
}

fn is_blank(choices: &[ChoiceRecord]) -> Result<Value, Box<HelperError>> {
    match is_blank_delta(choices) {
        Ok(blank) => Ok(Value::Bool(blank)),
        Err(error) => Err(failure(&error)),
    }
}

fn start_block(harness: &mut Harness, choices: &[ChoiceRecord]) -> Result<Value, Box<HelperError>> {
    match should_start_new_content_block(&mut harness.state, choices, &FixedIds) {
        Ok(started) => Ok(Value::Bool(started)),
        Err(error) => Err(failure(&error)),
    }
}

pub(super) fn answer(harness: &mut Harness, step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Snapshot => Ok(view::snapshot(&harness.state, harness.logged_id.as_deref())),
        Step::HoldStop { event } => hold_stop(harness, event),
        Step::InitialUsage => serde_json::to_value(initial_usage()).or_value_error(),
        Step::MergeHeldUsage { usage } => merge_held_usage(harness, usage),
        Step::Choiceless { usage } => choiceless(harness, usage.as_deref()),
        Step::Augment { event } => augment(harness, event),
        Step::RefusalStop { event } => refusal_stop(harness, event),
        Step::Compaction { calls } => compaction(harness, *calls),
        Step::DeltaHasContent { event } => Ok(Value::Bool(delta_has_content(event))),
        Step::IsBlank { choices } => is_blank(choices),
        Step::StartBlock { choices } => start_block(harness, choices),
    }
}
