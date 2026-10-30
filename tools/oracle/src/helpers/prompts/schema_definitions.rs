use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::error::DefinitionsError;
use betterlite_prompts::definitions::estimate::estimate_json_bytes;
use betterlite_prompts::definitions::expand::unpack_defs;
use betterlite_prompts::definitions::legacy::{
    DEFS_MAX_INLINED_BYTES, has_legacy_defs, unpack_legacy_defs,
};
use betterlite_prompts::definitions::sanitize::sanitize_for_anthropic;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    EstimateJsonBytes {
        value: Json,
    },
    UnpackDefs {
        schema: Object,
        defs: Object,
        #[serde(default)]
        max_inlined_bytes: Option<usize>,
    },
    HasLegacyDefs {
        schema: Json,
    },
    UnpackLegacyDefs {
        schema: Object,
        #[serde(default)]
        max_inlined_bytes: Option<usize>,
    },
    #[serde(rename = "sanitize_input_schema_for_anthropic")]
    SanitizeForAnthropic {
        input_schema: Object,
    },
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::EstimateJsonBytes { value } => Ok(Value::from(estimate_json_bytes(value))),
            Input::UnpackDefs {
                schema,
                defs,
                max_inlined_bytes,
            } => unpack_defs(schema.clone(), defs, *max_inlined_bytes)
                .map(object_to_value)
                .map_err(expansion_budget_error),
            Input::HasLegacyDefs { schema } => Ok(Value::from(has_legacy_defs(schema))),
            Input::UnpackLegacyDefs {
                schema,
                max_inlined_bytes,
            } => unpack_legacy_defs(
                schema.clone(),
                max_inlined_bytes.unwrap_or(DEFS_MAX_INLINED_BYTES),
            )
            .map(object_to_value)
            .map_err(expansion_budget_error),
            Input::SanitizeForAnthropic { input_schema } => {
                sanitize_for_anthropic(input_schema.clone())
                    .map(anthropic_schema_to_value)
                    .map_err(expansion_budget_error)
            }
        }
    }
}

fn json_to_value(json: Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(flag),
        Json::Number(number) => Value::Number(number),
        Json::Text(text) => Value::String(text),
        Json::List(items) => Value::Array(items.into_iter().map(json_to_value).collect()),
        Json::Object(fields) => object_to_value(fields),
    }
}

fn object_to_value(object: Object) -> Value {
    let entries: Map<String, Value> = object
        .into_iter()
        .map(|(key, value)| (key, json_to_value(value)))
        .collect();
    Value::Object(entries)
}

fn anthropic_schema_to_value(
    schema: betterlite_prompts::definitions::sanitize::AnthropicInputSchema,
) -> Value {
    let mut entries = Map::new();
    entries.insert("type".to_owned(), json_to_value(schema.schema_type));
    entries.insert("properties".to_owned(), json_to_value(schema.properties));
    if let Some(value) = schema.additional_properties {
        entries.insert("additionalProperties".to_owned(), json_to_value(value));
    }
    if let Some(value) = schema.required {
        entries.insert("required".to_owned(), json_to_value(value));
    }
    if let Some(value) = schema.defs {
        entries.insert("$defs".to_owned(), json_to_value(value));
    }
    if let Some(value) = schema.strict {
        entries.insert("strict".to_owned(), json_to_value(value));
    }
    Value::Object(entries)
}

fn expansion_budget_error(error: DefinitionsError) -> Box<HelperError> {
    let message = error.to_string();
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
