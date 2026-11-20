use betterlite_core::api::responses_input_content::is_truthy;
use serde_json::{Map, Value};

use super::{CACHE_CONTROL, TEXT, TYPE};

pub(super) fn text_block_row(block: &Value) -> Option<Value> {
    let fields = block.as_object()?;
    if fields.get(TYPE).and_then(Value::as_str) != Some(TEXT) {
        return None;
    }
    let Some(Value::String(text)) = fields.get(TEXT) else {
        return None;
    };
    if text.is_empty() {
        return None;
    }
    let mut row = Map::new();
    row.insert(TYPE.to_owned(), Value::String(TEXT.to_owned()));
    row.insert(TEXT.to_owned(), Value::String(text.clone()));
    if let Some(cache_control) = fields
        .get(CACHE_CONTROL)
        .filter(|&value| is_truthy(Some(value)))
    {
        row.insert(CACHE_CONTROL.to_owned(), cache_control.clone());
    }
    Some(Value::Object(row))
}

pub(super) fn text_block(text: String) -> Value {
    let mut block = Map::new();
    block.insert(TYPE.to_owned(), Value::String(TEXT.to_owned()));
    block.insert(TEXT.to_owned(), Value::String(text));
    Value::Object(block)
}
