use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, ItemsBoundary};

use super::ResponsesSseRecoveryError;
use super::lines::chunk_index;

const MESSAGE_TYPE: &str = "message";
const OUTPUT_TEXT_TYPE: &str = "output_text";
const ASSISTANT_ROLE: &str = "assistant";
const COMPLETED_STATUS: &str = "completed";
const MAX_CONTENT_INDEX: u16 = 1024_u16;

pub type OutputItems = IndexMap<i64, Json>;

pub type TextOnlyItems = IndexMap<i64, SyntheticItem>;

pub struct SyntheticItem {
    id: Json,
    content: Vec<OutputText>,
}

struct OutputText {
    text: String,
    annotations: Json,
}

// record_output_item_chunk: a real item is kept under output_index and a later chunk wins
pub fn record_output_item_chunk(
    parsed: &IndexMap<String, Json>,
    output_items: &mut OutputItems,
    limits: &ApiLimits,
) -> Result<(), ResponsesSseRecoveryError> {
    let Some(Json::Object(item)) = parsed.get("item") else {
        return Ok(());
    };
    let fallback = i64::try_from(output_items.len()).unwrap_or(i64::MAX);
    let output_index = chunk_index(parsed, "output_index", fallback);
    if !output_items.contains_key(&output_index) {
        let known_count = output_items.len().saturating_add(1);
        limits.check(ApiBoundariesInput::Items(ItemsBoundary { known_count }))?;
    }
    output_items.insert(output_index, Json::Object(item.clone()));
    Ok(())
}

// record_output_text_chunk: an output_text.done becomes a message item unless a real item holds its output_index
pub fn record_output_text_chunk(
    parsed: &IndexMap<String, Json>,
    output_items: &OutputItems,
    text_only_items: &mut TextOnlyItems,
    limits: &ApiLimits,
) -> Result<(), ResponsesSseRecoveryError> {
    let Some(Json::Text(text)) = parsed.get("text") else {
        return Ok(());
    };
    let fallback = i64::try_from(text_only_items.len()).unwrap_or(i64::MAX);
    let output_index = chunk_index(parsed, "output_index", fallback);
    if output_items.contains_key(&output_index) {
        return Ok(());
    }
    if !text_only_items.contains_key(&output_index) {
        let known_count = text_only_items.len().saturating_add(1);
        limits.check(ApiBoundariesInput::Items(ItemsBoundary { known_count }))?;
    }
    let item = text_only_items
        .entry(output_index)
        .or_insert_with(|| synthetic_item(parsed, output_index));
    let fallback = i64::try_from(item.content.len()).unwrap_or(i64::MAX);
    let content_index = chunk_index(parsed, "content_index", fallback);
    let Ok(content_index) = u16::try_from(content_index) else {
        return Ok(());
    };
    if content_index > MAX_CONTENT_INDEX {
        return Ok(());
    }
    let slot = usize::from(content_index);
    while item.content.len() <= slot {
        let known_count = item.content.len().saturating_add(1);
        limits.check(ApiBoundariesInput::Items(ItemsBoundary { known_count }))?;
        item.content.push(OutputText {
            text: String::new(),
            annotations: Json::List(Vec::new()),
        });
    }
    let Some(content_item) = item.content.get_mut(slot) else {
        return Ok(());
    };
    content_item.text = text.clone();
    match parsed.get("annotations") {
        None | Some(Json::Null) => {}
        Some(annotations) => content_item.annotations = annotations.clone(),
    }
    Ok(())
}

// _extract_output_from_completed_event: only a nonempty list under response.output is terminal output
pub fn completed_event_output(
    parsed: &IndexMap<String, Json>,
    limits: &ApiLimits,
) -> Result<Option<Vec<Json>>, ResponsesSseRecoveryError> {
    let Some(Json::Object(response)) = parsed.get("response") else {
        return Ok(None);
    };
    let Some(Json::List(output)) = response.get("output") else {
        return Ok(None);
    };
    if output.is_empty() {
        return Ok(None);
    }
    limits.check(ApiBoundariesInput::Items(ItemsBoundary {
        known_count: output.len(),
    }))?;
    Ok(Some(output.clone()))
}

// text-only items sit at the indices no real item took, and the merged order is the numeric index order
pub fn merged_output_items(
    output_items: &OutputItems,
    text_only_items: &TextOnlyItems,
    limits: &ApiLimits,
) -> Result<Vec<Json>, ResponsesSseRecoveryError> {
    let mut merged: OutputItems = OutputItems::new();
    for (index, item) in text_only_items {
        merged.insert(*index, item.to_json());
    }
    for (index, item) in output_items {
        merged.insert(*index, item.clone());
    }
    limits.check(ApiBoundariesInput::Items(ItemsBoundary {
        known_count: merged.len(),
    }))?;
    merged.sort_keys();
    Ok(merged.into_values().collect())
}

fn synthetic_item(parsed: &IndexMap<String, Json>, output_index: i64) -> SyntheticItem {
    let id = match parsed.get("item_id") {
        Some(id) if keeps_item_id(id) => id.clone(),
        None | Some(_) => Json::Text(format!("msg_{output_index}")),
    };
    SyntheticItem {
        id,
        content: Vec::new(),
    }
}

// the id falls back when Python's `item_id or f"msg_{output_index}"` would take the right side
fn keeps_item_id(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        // a nonzero number is the only numeric id Python keeps; the ordering comparison avoids clippy::float_cmp
        Json::Number(number) => number.as_f64().is_none_or(|value| value.abs() > 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

impl SyntheticItem {
    fn to_json(&self) -> Json {
        let content: Vec<Json> = self.content.iter().map(OutputText::to_json).collect();
        Json::Object(IndexMap::from([
            ("type".to_owned(), Json::Text(MESSAGE_TYPE.to_owned())),
            ("id".to_owned(), self.id.clone()),
            ("role".to_owned(), Json::Text(ASSISTANT_ROLE.to_owned())),
            ("status".to_owned(), Json::Text(COMPLETED_STATUS.to_owned())),
            ("content".to_owned(), Json::List(content)),
        ]))
    }
}

impl OutputText {
    fn to_json(&self) -> Json {
        Json::Object(IndexMap::from([
            ("type".to_owned(), Json::Text(OUTPUT_TEXT_TYPE.to_owned())),
            ("text".to_owned(), Json::Text(self.text.clone())),
            ("annotations".to_owned(), self.annotations.clone()),
        ]))
    }
}
