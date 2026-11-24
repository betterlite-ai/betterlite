use betterlite_interactions::types::json::Object as InteractionObject;
use betterlite_interactions::types::response::InteractionsStreamChunk;

use crate::api::responses_transform::response::{model_output_step, text_record};

// _build_interaction_start_event: pydantic's default object is "interaction", and only this event and the completion carry id and model.
#[must_use]
pub fn interaction_start_event(interaction_id: &str, model: &str) -> InteractionsStreamChunk {
    let mut event = chunk_with("interaction.created");
    event.id = Some(interaction_id.to_owned());
    event.model = Some(model.to_owned());
    event.status = Some(String::from("in_progress"));
    event
}

// _build_content_start_event: an empty model_output step, whose content array fills through step.delta events.
#[must_use]
pub fn content_start_event() -> InteractionsStreamChunk {
    let mut event = chunk_with("step.start");
    event.index = Some(0_i64);
    event.step = Some(model_output_step(Vec::new()));
    event
}

// _build_text_delta_event: one text delta at step index 0.
#[must_use]
pub fn text_delta_event(text: &str) -> InteractionsStreamChunk {
    let mut event = chunk_with("step.delta");
    event.index = Some(0_i64);
    event.delta = Some(text_record(text));
    event
}

// _build_content_stop_event: the step closes at index 0 with no other field set.
#[must_use]
pub fn content_stop_event() -> InteractionsStreamChunk {
    let mut event = chunk_with("step.stop");
    event.index = Some(0_i64);
    event
}

// _build_completion_event: one model_output step holding every delta collected so far.
#[must_use]
pub fn completion_event(interaction_id: &str, model: &str, text: &str) -> InteractionsStreamChunk {
    let mut event = chunk_with("interaction.completed");
    event.id = Some(interaction_id.to_owned());
    event.model = Some(model.to_owned());
    event.status = Some(String::from("completed"));
    event.steps = Some(vec![model_output_step(vec![text_record(text)])]);
    event
}

// Every field a synthesized event starts empty, and object keeps the pydantic default every Interactions event serializes.
fn chunk_with(event_type: &str) -> InteractionsStreamChunk {
    InteractionsStreamChunk {
        event_type: Some(event_type.to_owned()),
        id: None,
        object: Some(String::from("interaction")),
        model: None,
        agent: None,
        status: None,
        created: None,
        updated: None,
        outputs: None,
        steps: None,
        usage: None,
        delta: None,
        index: None,
        step: None,
        interaction_id: None,
        interaction: None,
        extra: InteractionObject::new(),
        hidden_params: InteractionObject::new(),
    }
}
