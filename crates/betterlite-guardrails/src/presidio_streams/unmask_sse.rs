use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
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

// Parses the JSON in raw as T, lets edit change it and writes it back; a value that is not a T stays as it was
fn edit_raw<T, Edit>(raw: &mut Box<RawValue>, edit: Edit) -> Result<(), serde_json::Error>
where
    T: DeserializeOwned + Serialize,
    Edit: FnOnce(&mut T) -> Result<(), serde_json::Error>,
{
    let Ok(mut value) = serde_json::from_str::<T>(raw.get()) else {
        return Ok(());
    };
    edit(&mut value)?;
    *raw = RawValue::from_string(serde_json::to_string(&value)?)?;
    Ok(())
}

fn edit_field<T, Edit>(fields: &mut Fields, key: &str, edit: Edit) -> Result<(), serde_json::Error>
where
    T: DeserializeOwned + Serialize,
    Edit: FnOnce(&mut T) -> Result<(), serde_json::Error>,
{
    fields
        .get_mut(key)
        .map_or(Ok(()), |raw| edit_raw::<T, _>(raw, edit))
}

// Applies edit to each list entry that is a JSON object and skips the rest
fn edit_objects<Edit>(
    entries: &mut [Box<RawValue>],
    mut edit: Edit,
) -> Result<(), serde_json::Error>
where
    Edit: FnMut(&mut Fields) -> Result<(), serde_json::Error>,
{
    for entry in entries {
        edit_raw::<Fields, _>(entry, &mut edit)?;
    }
    Ok(())
}

fn unmask_item(item: &mut Fields, tokens: &PiiTokens) -> Result<(), serde_json::Error> {
    edit_field::<Vec<Box<RawValue>>, _>(item, "content", |blocks| {
        edit_objects(blocks, |block| {
            edit_field::<String, _>(block, "text", |text| {
                *text = unmask_pii_text(text, tokens);
                Ok(())
            })
        })
    })
}

// LiteLLM's _unmask_responses_api_completed_chunk: unmasks the text of every content block of every output item of a response.completed event
pub fn unmask_completed_event(
    event: &mut Fields,
    tokens: &PiiTokens,
) -> Result<(), serde_json::Error> {
    edit_field::<Fields, _>(event, "response", |response| {
        edit_field::<Vec<Box<RawValue>>, _>(response, "output", |items| {
            edit_objects(items, |item| unmask_item(item, tokens))
        })
    })
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
