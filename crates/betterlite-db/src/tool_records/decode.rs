use std::collections::BTreeMap;

use serde_json::Number;

use crate::budget_permission_rows::JsonData;
use crate::resource_rows::{ResourceRowsError, json_value_is_set, parse_data};

use super::records::LiteLLMToolTableRow;

#[derive(Debug, thiserror::Error)]
pub enum ToolRecordDecodeError {
    #[error("the stored tool row is not a readable row")]
    Row {
        #[source]
        source: ResourceRowsError,
    },
    #[error("the stored tool row is not a JSON object")]
    NotAnObject,
    #[error("the stored tool row does not match the tool record")]
    Model {
        #[source]
        source: serde_json::Error,
    },
}

pub fn decode_tool_record(bytes: &[u8]) -> Result<LiteLLMToolTableRow, ToolRecordDecodeError> {
    let record = parse_data(bytes).map_err(|source| ToolRecordDecodeError::Row { source })?;
    let JsonData::Object(mut fields) = record else {
        return Err(ToolRecordDecodeError::NotAnObject);
    };
    apply_falsy_defaults(&mut fields);
    // LiteLLM's row converter never reads an owner column; the owner lookup fills the field afterwards
    fields.remove("user");
    let text = serde_json::to_string(&JsonData::Object(fields))
        .map_err(|source| ToolRecordDecodeError::Model { source })?;
    serde_json::from_str(&text).map_err(|source| ToolRecordDecodeError::Model { source })
}

// LiteLLM writes `row.get(policy) or "untrusted"` and `int(row.get("call_count") or 0)`
fn apply_falsy_defaults(fields: &mut BTreeMap<String, JsonData>) {
    for name in ["input_policy", "output_policy"] {
        if fields
            .get(name)
            .is_some_and(|value| !json_value_is_set(value))
        {
            fields.insert(name.to_owned(), JsonData::String(String::from("untrusted")));
        }
    }
    if fields
        .get("call_count")
        .is_some_and(|value| !json_value_is_set(value))
    {
        fields.insert(
            String::from("call_count"),
            JsonData::Number(Number::from(0)),
        );
    }
}
