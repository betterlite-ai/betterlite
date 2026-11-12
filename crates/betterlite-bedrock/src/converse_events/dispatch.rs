use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::converse_events::error::ConverseEventError;
use crate::converse_events::wire;

// The wrapper event names an InvokeModel Nova stream uses, tried in LiteLLM's order.
const NOVA_EVENT_TYPES: [&str; 6] = [
    "messageStart",
    "contentBlockStart",
    "contentBlockDelta",
    "contentBlockStop",
    "messageStop",
    "metadata",
];

// Any of these keys, even a falsey value, sends the chunk through the Converse parser.
const CONVERSE_KEYS: [&str; 4] = ["contentBlockIndex", "stopReason", "metrics", "trace"];

pub(super) enum BaseOutcome<'chunk> {
    Converse(&'chunk Json),
    Generic {
        text: &'chunk str,
        is_finished: bool,
        finish_reason: &'chunk str,
    },
}

pub(super) fn base_chunk<'chunk>(
    chunk: &'chunk Json,
    fields: &'chunk IndexMap<String, Json>,
    model: &str,
) -> Result<BaseOutcome<'chunk>, ConverseEventError> {
    let mut text = "";
    let mut is_finished = false;
    let mut finish_reason = "";
    if let Some(output) = fields.get("outputText") {
        text = wire::text(output, "outputText")?;
    } else if model.contains("ai21") {
        text = ai21_text(fields)?;
        is_finished = true;
        finish_reason = "stop";
    } else if CONVERSE_KEYS.iter().any(|key| fields.contains_key(*key)) {
        return Ok(BaseOutcome::Converse(chunk));
    } else if let Some(payload) = nova_payload(fields)? {
        return Ok(BaseOutcome::Converse(payload));
    } else if let Some((output_text, output_reason)) = outputs_text(fields)? {
        text = output_text;
        if let Some(reason) = output_reason {
            is_finished = true;
            finish_reason = reason;
        }
    } else if let Some(generation) = fields.get("generation") {
        text = wire::text(generation, "generation")?;
    } else if let Some(value) = fields.get("text") {
        text = wire::text(value, "text")?;
    } else if let Some(reason) = fields.get("finish_reason") {
        finish_reason = wire::text(reason, "finish_reason")?;
        is_finished = true;
    } else if let Some(reason) = fields
        .get("completionReason")
        .filter(|value| wire::non_empty(value))
    {
        is_finished = true;
        finish_reason = wire::text(reason, "completionReason")?;
    } else {
        // no recognized provider key leaves the chunk empty and unfinished
    }
    Ok(BaseOutcome::Generic {
        text,
        is_finished,
        finish_reason,
    })
}

fn ai21_text(fields: &IndexMap<String, Json>) -> Result<&str, ConverseEventError> {
    let completions = fields
        .get("completions")
        .ok_or(ConverseEventError::Missing {
            field: "completions",
        })?;
    let entries = wire::list(completions, "completions")?;
    let first = entries.first().ok_or(ConverseEventError::Missing {
        field: "completions[0]",
    })?;
    let entry = wire::object(first, "completions[0]")?;
    let data = entry.get("data").ok_or(ConverseEventError::Missing {
        field: "completions[0].data",
    })?;
    let data_fields = wire::object(data, "completions[0].data")?;
    wire::required_text(data_fields, "text", "completions[0].data.text")
}

fn outputs_text(
    fields: &IndexMap<String, Json>,
) -> Result<Option<(&str, Option<&str>)>, ConverseEventError> {
    let Some(outputs) = fields.get("outputs") else {
        return Ok(None);
    };
    let entries = wire::list(outputs, "outputs")?;
    let mut text = "";
    if entries.len() == 1 {
        let first = entries.first().ok_or(ConverseEventError::Missing {
            field: "outputs[0]",
        })?;
        let entry = wire::object(first, "outputs[0]")?;
        if let Some(value) = entry
            .get("text")
            .filter(|candidate| !matches!(candidate, Json::Null))
        {
            text = wire::text(value, "outputs[0].text")?;
        }
    }
    let reason = match fields
        .get("stop_reason")
        .filter(|candidate| !matches!(candidate, Json::Null))
    {
        Some(value) => Some(wire::text(value, "stop_reason")?),
        None => None,
    };
    Ok(Some((text, reason)))
}

fn nova_payload(fields: &IndexMap<String, Json>) -> Result<Option<&Json>, ConverseEventError> {
    for name in NOVA_EVENT_TYPES {
        if let Some(value) = fields.get(name) {
            wire::object(value, "nova event payload")?;
            return Ok(Some(value));
        }
    }
    Ok(None)
}
