use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::controls::multipart::{
    DEFAULT_MAX_RECURSE_DEPTH, FormField, FormValue, flatten_form_field_values,
    serialize_multipart_fields,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Flatten(FlattenInput),
    Serialize(SerializeInput),
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FlattenInput {
    #[serde(default)]
    pub(crate) sources: Vec<Option<JsonValue>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SerializeInput {
    pub(crate) data: JsonValue,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::Flatten(fields) => flatten(fields),
            Input::Serialize(fields) => serialize_fields(fields),
        }
    }
}

fn flatten(fields: &FlattenInput) -> crate::helpers::HelperResult {
    let sources: Vec<Option<&JsonValue>> = fields.sources.iter().map(Option::as_ref).collect();
    let flattened =
        flatten_form_field_values(&sources, DEFAULT_MAX_RECURSE_DEPTH).or_value_error()?;
    let pairs = flattened.into_iter().map(render_form_field);
    Ok(render_fields(pairs.collect()))
}

fn serialize_fields(fields: &SerializeInput) -> crate::helpers::HelperResult {
    let serialized =
        serialize_multipart_fields(&fields.data, DEFAULT_MAX_RECURSE_DEPTH).or_value_error()?;
    let pairs = serialized
        .into_iter()
        .map(|field| (field.name, json!([Value::Null, field.value])));
    Ok(render_fields(pairs.collect()))
}

fn render_form_field(field: FormField) -> (String, Value) {
    let value = match field.value {
        FormValue::Single(text) => json!(text),
        FormValue::Repeated(texts) => json!(texts),
    };
    (field.name, value)
}

fn render_fields(pairs: Vec<(String, Value)>) -> Value {
    let rendered: Vec<Value> = pairs
        .into_iter()
        .map(|(name, value)| json!([name, value]))
        .collect();
    json!({"fields": rendered})
}
