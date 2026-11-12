use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use num_traits::ToPrimitive as _;
use serde_json::Number;

const COMPACTION_EDIT_TYPE: &str = "compact_20260112";
const INPUT_TOKENS_TRIGGER_TYPE: &str = "input_tokens";

#[derive(Debug, thiserror::Error)]
pub enum ContextManagementError {
    #[error("the compact threshold is not a whole number of tokens")]
    CompactThresholdNotWhole,
}

pub fn map_context_management(
    context_management: &Json,
) -> Result<Option<IndexMap<String, Json>>, ContextManagementError> {
    match context_management {
        Json::Object(fields) if fields.contains_key("edits") => Ok(Some(fields.clone())),
        Json::List(entries) => {
            let mut edits = Vec::new();
            for entry in entries {
                let Json::Object(fields) = entry else {
                    continue;
                };
                if fields.get("type").and_then(text_of) == Some("compaction") {
                    edits.push(compact_edit(fields)?);
                }
            }
            Ok((!edits.is_empty())
                .then(|| IndexMap::from([("edits".to_owned(), Json::List(edits))])))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => Ok(None),
    }
}

const fn text_of(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn compact_edit(fields: &IndexMap<String, Json>) -> Result<Json, ContextManagementError> {
    let mut edit = IndexMap::new();
    edit.insert(
        "type".to_owned(),
        Json::Text(COMPACTION_EDIT_TYPE.to_owned()),
    );
    if let Some(threshold) = fields
        .get("compact_threshold")
        .filter(|value| !matches!(value, Json::Null))
        && let Some(tokens) = compact_threshold_tokens(threshold)?
    {
        let mut trigger = IndexMap::new();
        trigger.insert(
            "type".to_owned(),
            Json::Text(INPUT_TOKENS_TRIGGER_TYPE.to_owned()),
        );
        trigger.insert("value".to_owned(), Json::Number(tokens));
        edit.insert("trigger".to_owned(), Json::Object(trigger));
    }
    for (key, value) in fields {
        if key != "type" && key != "compact_threshold" {
            edit.insert(key.clone(), value.clone());
        }
    }
    Ok(Json::Object(edit))
}

fn compact_threshold_tokens(threshold: &Json) -> Result<Option<Number>, ContextManagementError> {
    match threshold {
        Json::Bool(value) => Ok(Some(Number::from(i64::from(*value)))),
        Json::Number(number) => {
            if number.is_i64() || number.is_u64() {
                return Ok(Some(number.clone()));
            }
            let float = number
                .as_f64()
                .ok_or(ContextManagementError::CompactThresholdNotWhole)?;
            let tokens = float
                .trunc()
                .to_i64()
                .ok_or(ContextManagementError::CompactThresholdNotWhole)?;
            Ok(Some(Number::from(tokens)))
        }
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => Ok(None),
    }
}
