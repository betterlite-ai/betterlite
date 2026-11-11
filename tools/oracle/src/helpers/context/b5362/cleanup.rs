use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::reasoning_aliases::{
    peek_reasoning_summary_aliases, strip_reasoning_aliases,
};
use betterlite_context::non_default_params::schema_cleanup::{
    remove_additional_properties, remove_strict_from_schema,
};
use betterlite_models::controls::json_value::JsonValue;
use serde::Deserialize;
use serde_json::{Value, json};

use super::Failure;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CleanupCall {
    RemoveAdditionalProperties { schema: JsonValue },
    RemoveStrict { schema: JsonValue },
    PeekAliases { optional_params: Params },
    StripAliases { optional_params: Params },
}

pub(super) fn run(call: &CleanupCall) -> Result<Value, Failure> {
    match call {
        CleanupCall::RemoveAdditionalProperties { schema } => {
            let mut cleaned = schema.clone();
            remove_additional_properties(&mut cleaned);
            Ok(serde_json::to_value(cleaned)?)
        }
        CleanupCall::RemoveStrict { schema } => {
            let mut cleaned = schema.clone();
            remove_strict_from_schema(&mut cleaned);
            Ok(serde_json::to_value(cleaned)?)
        }
        CleanupCall::PeekAliases { optional_params } => Ok(serde_json::to_value(
            peek_reasoning_summary_aliases(optional_params),
        )?),
        CleanupCall::StripAliases { optional_params } => {
            let (stripped, summary) = strip_reasoning_aliases(optional_params);
            Ok(json!({ "params": stripped, "summary": summary }))
        }
    }
}
