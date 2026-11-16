use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;

use super::error::ContentError;
use super::events::{ContentEvent, ResponseDelta};
use super::ids::ContentIds;

pub fn content_delta_event(
    message: &Json,
    output_item_id: &str,
    response_id: &str,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> Result<ContentEvent, ContentError> {
    let delta = concatenated_delta(message)?;
    let event = ResponseDelta {
        content_index: 0,
        event_id: format!("event_{}", ids.next_id()),
        item_id: output_item_id.to_owned(),
        output_index: 0,
        response_id: response_id.to_owned(),
        delta,
    };
    Ok(match delta_type {
        AllDeltaType::Text => ContentEvent::TextDelta(event),
        AllDeltaType::Audio => ContentEvent::AudioDelta(event),
    })
}

fn concatenated_delta(message: &Json) -> Result<String, ContentError> {
    let Some(fields) = message.as_object() else {
        return Err(ContentError::ContentNotObject);
    };
    let Some(model_turn) = fields.get("modelTurn") else {
        return Ok(String::new());
    };
    let Some(turn) = model_turn.as_object() else {
        return Err(ContentError::ModelTurnNotObject);
    };
    let parts = match turn.get("parts") {
        None => return Ok(String::new()),
        Some(Json::List(parts)) => parts,
        Some(_) => return Err(ContentError::PartsNotList),
    };
    let mut delta = String::new();
    for part in parts {
        append_part_delta(&mut delta, part)?;
    }
    Ok(delta)
}

fn append_part_delta(delta: &mut String, part: &Json) -> Result<(), ContentError> {
    let Some(part) = part.as_object() else {
        return Err(ContentError::PartNotObject);
    };
    if let Some(text) = part.get("text") {
        let Json::Text(text) = text else {
            return Err(ContentError::PartValueNotString);
        };
        delta.push_str(text);
        return Ok(());
    }
    let Some(inline_data) = part.get("inlineData") else {
        return Ok(());
    };
    let Some(inline_data) = inline_data.as_object() else {
        return Err(ContentError::PartNotObject);
    };
    match inline_data.get("data") {
        None => Ok(()),
        Some(Json::Text(data)) => {
            delta.push_str(data);
            Ok(())
        }
        Some(_) => Err(ContentError::PartValueNotString),
    }
}
