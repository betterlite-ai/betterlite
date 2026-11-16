use indexmap::IndexMap;
use serde_json::value::RawValue;

use crate::presidio_actions::tokens::unmask_pii_text;
use crate::presidio_response::wire::to_raw;

pub type Fields = IndexMap<String, Box<RawValue>>;
pub type PiiTokens = IndexMap<String, String>;

// A data line is rewritten only when it is a text_delta whose text holds a token, and any line that does not parse stays as it was
fn unmask_data_line(raw_json: &str, tokens: &PiiTokens) -> Option<String> {
    let text_field = |fields: &Fields, key: &str| -> Option<String> {
        serde_json::from_str(fields.get(key)?.get()).ok()
    };
    let mut event: Fields = serde_json::from_str(raw_json).ok()?;
    if text_field(&event, "type")? != "content_block_delta" {
        return None;
    }
    let mut delta: Fields = serde_json::from_str(event.get("delta")?.get()).ok()?;
    if text_field(&delta, "type")? != "text_delta" {
        return None;
    }
    let text = text_field(&delta, "text")?;
    let unmasked = unmask_pii_text(&text, tokens);
    if unmasked == text {
        return None;
    }
    delta.insert("text".to_owned(), to_raw(&unmasked).ok()?);
    event.insert("delta".to_owned(), to_raw(&delta).ok()?);
    Some(format!("data: {}", serde_json::to_string(&event).ok()?))
}

// Unmasks the text_delta events of one raw Anthropic SSE chunk; lines come back without their trailing CR, as LiteLLM rejoins them
#[must_use]
pub fn unmask_sse_bytes_chunk(chunk: &[u8], tokens: &PiiTokens) -> Vec<u8> {
    let Ok(text) = str::from_utf8(chunk) else {
        return chunk.to_vec();
    };
    let lines: Vec<String> = text
        .split('\n')
        .map(|raw_line| {
            let line = raw_line.trim_end_matches('\r');
            line.strip_prefix("data: ")
                .and_then(|raw_json| unmask_data_line(raw_json, tokens))
                .unwrap_or_else(|| line.to_owned())
        })
        .collect();
    lines.join("\n").into_bytes()
}

// Applies edit to every entry of the JSON array in raw and writes the array back when edit changed one entry; a value that is not an array stays as it was
fn edit_list<Edit>(raw: &mut Box<RawValue>, mut edit: Edit) -> Result<bool, serde_json::Error>
where
    Edit: FnMut(&mut Box<RawValue>) -> Result<bool, serde_json::Error>,
{
    let Ok(mut entries) = serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) else {
        return Ok(false);
    };
    let mut changed = false;
    for entry in &mut entries {
        changed |= edit(entry)?;
    }
    if !changed {
        return Ok(false);
    }
    *raw = RawValue::from_string(serde_json::to_string(&entries)?)?;
    Ok(true)
}

// Unmasks the text of one content block; true when the text changed
fn unmask_block(block: &mut Box<RawValue>, tokens: &PiiTokens) -> Result<bool, serde_json::Error> {
    let Ok(mut fields) = serde_json::from_str::<Fields>(block.get()) else {
        return Ok(false);
    };
    let Some(text) = fields.get_mut("text") else {
        return Ok(false);
    };
    let Ok(found) = serde_json::from_str::<String>(text.get()) else {
        return Ok(false);
    };
    let unmasked = unmask_pii_text(&found, tokens);
    if unmasked == found {
        return Ok(false);
    }
    *text = to_raw(&unmasked)?;
    *block = RawValue::from_string(serde_json::to_string(&fields)?)?;
    Ok(true)
}

// Unmasks the content blocks of one output item; true when a text changed
fn unmask_output_item(
    item: &mut Box<RawValue>,
    tokens: &PiiTokens,
) -> Result<bool, serde_json::Error> {
    let Ok(mut fields) = serde_json::from_str::<Fields>(item.get()) else {
        return Ok(false);
    };
    let Some(content) = fields.get_mut("content") else {
        return Ok(false);
    };
    if !edit_list(content, |block| unmask_block(block, tokens))? {
        return Ok(false);
    }
    *item = RawValue::from_string(serde_json::to_string(&fields)?)?;
    Ok(true)
}

// LiteLLM's _unmask_responses_api_completed_chunk: unmasks the text of every content block of every output item of a response.completed event
pub fn unmask_completed_event(
    event: &mut Fields,
    tokens: &PiiTokens,
) -> Result<(), serde_json::Error> {
    let Some(response) = event.get_mut("response") else {
        return Ok(());
    };
    let Ok(mut fields) = serde_json::from_str::<Fields>(response.get()) else {
        return Ok(());
    };
    let Some(output) = fields.get_mut("output") else {
        return Ok(());
    };
    if !edit_list(output, |item| unmask_output_item(item, tokens))? {
        return Ok(());
    }
    *response = RawValue::from_string(serde_json::to_string(&fields)?)?;
    Ok(())
}

// Unmasks a response.completed event and leaves any other event alone
pub fn unmask_other_event(
    event: Box<RawValue>,
    tokens: &PiiTokens,
) -> Result<Box<RawValue>, serde_json::Error> {
    let Ok(mut fields) = serde_json::from_str::<Fields>(event.get()) else {
        return Ok(event);
    };
    let event_type = fields
        .get("type")
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok());
    if event_type.as_deref() != Some("response.completed") {
        return Ok(event);
    }
    unmask_completed_event(&mut fields, tokens)?;
    RawValue::from_string(serde_json::to_string(&fields)?)
}
